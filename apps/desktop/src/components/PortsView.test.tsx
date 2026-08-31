import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { I18nProvider } from "../i18n";
import type { DashboardSnapshot } from "../types";
import { PortsView } from "./PortsView";

const snapshot: DashboardSnapshot = {
  generatedAt: 1,
  scanError: null,
  privilege: { elevated: false, elevationAvailable: true, monitorOnly: false },
  settings: {
    pollIntervalMs: 2_000,
    logCapacity: 500,
    languagePreference: "en",
    closeBehavior: "ask",
    archiveRunLogs: false,
  },
  runLogArchive: { enabled: false, available: true, unavailableReason: null },
  restoreSet: { profileIds: [] },
  launchGroups: [],
  projects: [],
  ports: [{
    port: 5173,
    protocol: "tcp",
    state: "LISTEN",
    bindAddress: "127.0.0.1",
    isPublic: false,
    active: true,
    pid: 42,
    processName: "node.exe",
    executablePath: "C:\\Program Files\\nodejs\\node.exe",
    commandLine: "node vite",
    processStartedAt: 1,
  }],
};

function renderPorts() {
  render(
    <I18nProvider>
      <PortsView
        snapshot={snapshot}
        busyProfileIds={new Set()}
        onOpenPort={vi.fn()}
        onTerminate={vi.fn()}
        onConfirmAssociation={vi.fn()}
        onStartProfile={vi.fn()}
      />
    </I18nProvider>,
  );
}

describe("PortsView clipboard controls", () => {
  const originalClipboard = navigator.clipboard;

  afterEach(() => {
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: originalClipboard });
  });

  it("copies PID, executable, and command line from expanded details", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    const user = userEvent.setup();
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    renderPorts();

    await user.click(screen.getByRole("button", { name: "View details for port 5173" }));
    await user.click(screen.getByRole("button", { name: "Copy PID 42" }));
    await user.click(screen.getByRole("button", { name: "Copy executable path" }));
    await user.click(screen.getByRole("button", { name: "Copy command line" }));

    expect(writeText.mock.calls.map(([value]) => value)).toEqual([
      "42",
      "C:\\Program Files\\nodejs\\node.exe",
      "node vite",
    ]);
  });

  it("surfaces clipboard permission failures in the port detail", async () => {
    const user = userEvent.setup();
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: vi.fn().mockRejectedValue(new Error("permission denied")) },
    });
    renderPorts();

    await user.click(screen.getByRole("button", { name: "View details for port 5173" }));
    await user.click(screen.getByRole("button", { name: "Copy PID 42" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("permission denied");
  });
});

describe("PortsView conflict focus", () => {
  it("filters an exact port and protocol when TCP and UDP share a number", async () => {
    const dualProtocolSnapshot: DashboardSnapshot = {
      ...snapshot,
      ports: [
        snapshot.ports[0],
        { ...snapshot.ports[0], protocol: "udp", state: "BOUND", pid: 43 },
      ],
    };

    render(
      <I18nProvider>
        <PortsView
          snapshot={dualProtocolSnapshot}
          busyProfileIds={new Set()}
          onOpenPort={vi.fn()}
          onTerminate={vi.fn()}
          onConfirmAssociation={vi.fn()}
          onStartProfile={vi.fn()}
          focusRequest={{ port: 5173, protocol: "tcp", nonce: 1 }}
        />
      </I18nProvider>,
    );

    expect(screen.getByPlaceholderText("Search ports")).toHaveValue("5173 tcp");
    expect(screen.getByText("/tcp")).toBeInTheDocument();
    expect(screen.queryByText("/udp")).not.toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Port 5173 details" })).toBeInTheDocument();
  });

  it("finishes the focus highlight even when a polling snapshot rerenders the view", () => {
    vi.useFakeTimers();
    const handled = vi.fn();
    const props: React.ComponentProps<typeof PortsView> = {
      snapshot,
      busyProfileIds: new Set(),
      onOpenPort: vi.fn(),
      onTerminate: vi.fn(),
      onConfirmAssociation: vi.fn(),
      onStartProfile: vi.fn(),
      focusRequest: { port: 5173, protocol: "tcp", nonce: 1 },
      onFocusHandled: handled,
    };
    const view = render(<I18nProvider><PortsView {...props} /></I18nProvider>);

    expect(document.querySelector("tr.is-focused-port")).not.toBeNull();
    act(() => vi.advanceTimersByTime(2_000));
    const handledAfterRerender = vi.fn();
    view.rerender(
      <I18nProvider>
        <PortsView
          {...props}
          snapshot={{ ...snapshot, generatedAt: 2_001 }}
          onFocusHandled={handledAfterRerender}
        />
      </I18nProvider>,
    );
    act(() => vi.advanceTimersByTime(500));

    expect(handled).not.toHaveBeenCalled();
    expect(handledAfterRerender).toHaveBeenCalledTimes(1);
    expect(document.querySelector("tr.is-focused-port")).toBeNull();
    vi.useRealTimers();
  });
});
