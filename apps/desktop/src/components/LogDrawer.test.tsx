import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { I18nProvider } from "../i18n";
import type {
  LaunchProfile,
  Project,
  RunCoveApi,
  RunLogArchiveState,
  RunLogEvent,
} from "../types";
import { LogDrawer } from "./LogDrawer";

const profile: LaunchProfile = {
  id: "profile-web",
  projectId: "project-web",
  name: "Web",
  program: "npm.cmd",
  args: ["run", "dev"],
  cwd: "D:\\projects\\web",
  expectedPorts: [],
  status: "running",
  pid: 1200,
};

const project: Project = {
  id: "project-web",
  name: "Web project",
  path: "D:\\projects\\web",
  profiles: [profile],
  createdAt: 1,
  updatedAt: 1,
};

const log: RunLogEvent = {
  profileId: profile.id,
  stream: "stdout",
  line: "ready",
  timestamp: 1,
};

function logApi(overrides: Partial<RunCoveApi>): RunCoveApi {
  return {
    getLogs: vi.fn().mockResolvedValue([]),
    onRunLog: vi.fn().mockResolvedValue(() => undefined),
    clearLogs: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  } as unknown as RunCoveApi;
}

const archiveOff: RunLogArchiveState = { enabled: false, available: true, unavailableReason: null };

/** The archive is off and healthy unless a test says otherwise. */
const archiveProps = {
  archive: archiveOff,
  onToggleArchive: async (enabled: boolean) => ({ ...archiveOff, enabled }),
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((complete) => {
    resolve = complete;
  });
  return { promise, resolve };
}

describe("LogDrawer failures", () => {
  it("leaves loading state and offers a retry when log loading fails", async () => {
    const getLogs = vi.fn().mockRejectedValue(new Error("IPC unavailable"));
    render(<LogDrawer api={logApi({ getLogs })} profile={profile} project={project} capacity={100} {...archiveProps} onClose={vi.fn()} />);

    expect(await screen.findByRole("alert")).toHaveTextContent("Logs could not be loaded: IPC unavailable");
    expect(screen.queryByText("Loading logs...")).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "Retry loading logs" }));
    expect(getLogs).toHaveBeenCalledTimes(2);
  });

  it("keeps existing logs visible when clearing fails", async () => {
    const clearLogs = vi.fn().mockRejectedValue(new Error("database busy"));
    render(
      <LogDrawer
        api={logApi({ getLogs: vi.fn().mockResolvedValue([log]), clearLogs })}
        profile={profile}
        project={project}
        capacity={100}
        {...archiveProps}
        onClose={vi.fn()}
      />,
    );

    expect(await screen.findByText("ready")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Clear session logs" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Logs could not be cleared: database busy");
    expect(screen.getByText("ready")).toBeInTheDocument();
  });

  it("keeps live log lines that arrive before the initial history finishes loading", async () => {
    const history = deferred<RunLogEvent[]>();
    let emit: ((event: RunLogEvent) => void) | undefined;
    render(
      <LogDrawer
        api={logApi({
          getLogs: vi.fn().mockReturnValue(history.promise),
          onRunLog: vi.fn().mockImplementation(async (handler) => {
            emit = handler;
            return () => undefined;
          }),
        })}
        profile={profile}
        project={project}
        capacity={100}
        {...archiveProps}
        onClose={vi.fn()}
      />,
    );

    await waitFor(() => expect(emit).toBeTypeOf("function"));
    act(() => emit?.({ ...log, line: "live", timestamp: 2 }));
    expect(screen.getByText("live")).toBeInTheDocument();

    history.resolve([{ ...log, line: "history" }]);
    await act(async () => {
      await history.promise;
    });

    expect(screen.getByText("history")).toBeInTheDocument();
    expect(screen.getByText("live")).toBeInTheDocument();
  });

  it("keeps genuine duplicate lines when history overlaps the live stream", async () => {
    const history = deferred<RunLogEvent[]>();
    let emit: ((event: RunLogEvent) => void) | undefined;
    render(
      <LogDrawer
        api={logApi({
          getLogs: vi.fn().mockReturnValue(history.promise),
          onRunLog: vi.fn().mockImplementation(async (handler) => {
            emit = handler;
            return () => undefined;
          }),
        })}
        profile={profile}
        project={project}
        capacity={100}
        {...archiveProps}
        onClose={vi.fn()}
      />,
    );

    await waitFor(() => expect(emit).toBeTypeOf("function"));
    act(() => {
      emit?.(log);
      emit?.(log);
    });
    expect(screen.getAllByText("ready")).toHaveLength(2);

    history.resolve([log]);
    await act(async () => {
      await history.promise;
    });

    expect(screen.getAllByText("ready")).toHaveLength(2);
  });

  it("does not lose a line emitted after the history snapshot but before subscription setup", async () => {
    const subscription = deferred<() => void>();
    const bufferedLogs = [{ ...log, line: "history" }];
    const getLogs = vi.fn().mockImplementation(async () => [...bufferedLogs]);
    const onRunLog = vi.fn().mockReturnValue(subscription.promise);

    render(
      <LogDrawer
        api={logApi({ getLogs, onRunLog })}
        profile={profile}
        project={project}
        capacity={100}
        {...archiveProps}
        onClose={vi.fn()}
      />,
    );

    await waitFor(() => expect(onRunLog).toHaveBeenCalledTimes(1));
    await act(async () => undefined);
    bufferedLogs.push({ ...log, line: "during subscription", timestamp: 2 });
    subscription.resolve(() => undefined);

    expect(await screen.findByText("history")).toBeInTheDocument();
    expect(await screen.findByText("during subscription")).toBeInTheDocument();
    expect(getLogs).toHaveBeenCalledTimes(1);
  });

  it("disposes a delayed subscription without reading history after unmount", async () => {
    const subscription = deferred<() => void>();
    const dispose = vi.fn();
    const getLogs = vi.fn().mockResolvedValue([]);
    const onRunLog = vi.fn().mockReturnValue(subscription.promise);
    const view = render(
      <LogDrawer
        api={logApi({ getLogs, onRunLog })}
        profile={profile}
        project={project}
        capacity={100}
        {...archiveProps}
        onClose={vi.fn()}
      />,
    );

    await waitFor(() => expect(onRunLog).toHaveBeenCalledTimes(1));
    view.unmount();
    subscription.resolve(dispose);
    await act(async () => {
      await subscription.promise;
    });

    expect(dispose).toHaveBeenCalledTimes(1);
    expect(getLogs).not.toHaveBeenCalled();
  });

  it("reloads logs after clearing so lines emitted during the command are retained", async () => {
    const getLogs = vi.fn()
      .mockResolvedValueOnce([log])
      .mockResolvedValueOnce([{ ...log, line: "after clear", timestamp: 2 }]);
    render(
      <LogDrawer
        api={logApi({ getLogs })}
        profile={profile}
        project={project}
        capacity={100}
        {...archiveProps}
        onClose={vi.fn()}
      />,
    );

    expect(await screen.findByText("ready")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Clear session logs" }));

    expect(await screen.findByText("after clear")).toBeInTheDocument();
    expect(getLogs).toHaveBeenCalledTimes(2);
  });

  it("localizes system stream labels and clipboard availability failures", async () => {
    window.localStorage.setItem("runcove.language", "zh-CN");
    const clipboard = navigator.clipboard;
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: undefined });
    const systemLog: RunLogEvent = { ...log, stream: "system" };

    try {
      render(
        <I18nProvider>
          <LogDrawer
            api={logApi({ getLogs: vi.fn().mockResolvedValue([systemLog]) })}
            profile={profile}
            project={project}
            capacity={100}
            {...archiveProps}
            onClose={vi.fn()}
          />
        </I18nProvider>,
      );

      await screen.findByText("ready");
      expect(screen.getAllByText("系统")).toHaveLength(2);
      fireEvent.click(screen.getByRole("button", { name: "复制当前日志" }));
      expect(await screen.findByRole("alert")).toHaveTextContent(
        "无法复制日志：当前无法访问剪贴板",
      );
    } finally {
      Object.defineProperty(navigator, "clipboard", { configurable: true, value: clipboard });
    }
  });
  it("shows a lifecycle line in the window's language and copies what it shows", async () => {
    window.localStorage.setItem("runcove.language", "zh-CN");
    const clipboard = navigator.clipboard;
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    const lifecycle: RunLogEvent = {
      ...log,
      stream: "system",
      line: "Process exited unexpectedly with exit code 7",
      reason: { kind: "exited-unexpectedly", code: 7 },
    };

    try {
      render(
        <I18nProvider>
          <LogDrawer
            api={logApi({ getLogs: vi.fn().mockResolvedValue([lifecycle, { ...log, timestamp: 2 }]) })}
            profile={profile}
            project={project}
            capacity={100}
            {...archiveProps}
            onClose={vi.fn()}
          />
        </I18nProvider>,
      );

      await screen.findByText("进程异常退出，退出码 7");
      expect(screen.queryByText(lifecycle.line)).not.toBeInTheDocument();
      // The child process's own output is never rewritten.
      expect(screen.getByText("ready")).toBeInTheDocument();

      fireEvent.click(screen.getByRole("button", { name: "复制当前日志" }));
      await waitFor(() => expect(writeText).toHaveBeenCalledWith(
        "[system] 进程异常退出，退出码 7\n[stdout] ready",
      ));
    } finally {
      Object.defineProperty(navigator, "clipboard", { configurable: true, value: clipboard });
    }
  });
});

describe("LogDrawer run log archiving", () => {
  const archiveOn: RunLogArchiveState = { enabled: true, available: true, unavailableReason: null };

  function renderDrawer(
    archive: RunLogArchiveState,
    onToggleArchive: (enabled: boolean) => Promise<RunLogArchiveState>,
  ) {
    return render(
      <LogDrawer
        api={logApi({})}
        profile={profile}
        project={project}
        capacity={100}
        archive={archive}
        onToggleArchive={onToggleArchive}
        onClose={vi.fn()}
      />,
    );
  }

  it("warns about the files it writes before anything is enabled", () => {
    renderDrawer(archiveOff, vi.fn());

    const toggle = screen.getByRole("checkbox", { name: "Archive run logs" });
    expect(toggle).not.toBeChecked();
    expect(screen.getByText(/can contain tokens your own services print/)).toBeInTheDocument();
    // The hint is what the checkbox points at, so a screen reader hears the warning
    // with the control rather than only on the way past it.
    expect(toggle).toHaveAccessibleDescription(/can contain tokens your own services print/);
  });

  it("promises nothing when enabling produced an archive that cannot run", async () => {
    const onToggleArchive = vi.fn().mockResolvedValue({
      enabled: true,
      available: false,
      unavailableReason: "the data folder is read-only",
    });
    renderDrawer(archiveOff, onToggleArchive);

    await userEvent.click(screen.getByRole("checkbox", { name: "Archive run logs" }));

    expect(onToggleArchive).toHaveBeenCalledWith(true);
    await waitFor(() => expect(screen.getByRole("checkbox", { name: "Archive run logs" })).toBeEnabled());
    expect(screen.queryByText("Runs started from now on will be archived.")).not.toBeInTheDocument();
  });

  it("confirms an enabled archive only when the backend says it is running", async () => {
    const onToggleArchive = vi.fn().mockResolvedValue(archiveOn);
    renderDrawer(archiveOff, onToggleArchive);

    await userEvent.click(screen.getByRole("checkbox", { name: "Archive run logs" }));

    expect(await screen.findByText("Runs started from now on will be archived.")).toBeInTheDocument();
  });

  it("says that disabling leaves open archives to finish", async () => {
    const onToggleArchive = vi.fn().mockResolvedValue(archiveOff);
    renderDrawer(archiveOn, onToggleArchive);

    const toggle = screen.getByRole("checkbox", { name: "Archive run logs" });
    expect(toggle).toBeChecked();
    await userEvent.click(toggle);

    expect(onToggleArchive).toHaveBeenCalledWith(false);
    expect(
      await screen.findByText("New runs will not be archived. Archives already open are being finished."),
    ).toBeInTheDocument();
  });

  it("reports a failed toggle without borrowing the log retry", async () => {
    const onToggleArchive = vi.fn().mockRejectedValue(new Error("settings are locked"));
    renderDrawer(archiveOff, onToggleArchive);

    await userEvent.click(screen.getByRole("checkbox", { name: "Archive run logs" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Run log archiving could not be changed: settings are locked",
    );
    expect(screen.queryByRole("button", { name: "Retry loading logs" })).not.toBeInTheDocument();
  });

  it("keeps the toggle usable while the archive is unavailable, because it is the retry", async () => {
    const onToggleArchive = vi.fn().mockResolvedValue(archiveOff);
    renderDrawer(
      { enabled: true, available: false, unavailableReason: "the data folder is read-only" },
      onToggleArchive,
    );

    expect(screen.getByRole("alert")).toHaveTextContent(
      "Run log archiving is unavailable this session: the data folder is read-only",
    );
    // Checked is the persisted preference; the alert above is what says nothing is
    // being captured. Disabling the control would strand the user with a preference
    // they cannot clear and no way to retry.
    const toggle = screen.getByRole("checkbox", { name: "Archive run logs" });
    expect(toggle).toBeChecked();
    expect(toggle).toBeEnabled();

    await userEvent.click(toggle);
    expect(onToggleArchive).toHaveBeenCalledWith(false);
  });

  it("falls back to a generic warning when no reason came back", () => {
    renderDrawer({ enabled: true, available: false }, vi.fn());

    expect(screen.getByRole("alert")).toHaveTextContent(
      "Run log archiving could not be started this session.",
    );
  });
});
