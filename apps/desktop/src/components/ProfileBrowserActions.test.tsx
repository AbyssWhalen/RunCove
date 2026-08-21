import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { DashboardSnapshot, LaunchProfile, Project } from "../types";
import { OverviewView } from "./OverviewView";
import { ProjectsView } from "./ProjectsView";

const udpProject: Project = {
  id: "project-udp",
  name: "UDP Service",
  path: "D:\\projects\\udp-service",
  profiles: [{
    id: "profile-udp",
    projectId: "project-udp",
    name: "UDP only",
    program: "node.exe",
    args: ["server.js"],
    cwd: "D:\\projects\\udp-service",
    expectedPorts: [{
      id: "port-udp",
      profileId: "profile-udp",
      port: 5353,
      protocol: "udp",
    }],
    status: "running",
    pid: 1234,
  }],
  createdAt: 1,
  updatedAt: 1,
};

const snapshot: DashboardSnapshot = {
  ports: [],
  projects: [udpProject],
  restoreSet: { profileIds: [], savedAt: null },
  privilege: { elevated: false, elevationAvailable: true, monitorOnly: false },
  settings: {
    pollIntervalMs: 2_000,
    logCapacity: 1_000,
    languagePreference: "system",
    closeBehavior: "ask",
    archiveRunLogs: false,
  },
  runLogArchive: { enabled: false, available: true, unavailableReason: null },
  generatedAt: 1,
};

function commonActions(onOpenPort: (profile: LaunchProfile) => void) {
  return {
    busyProfileIds: new Set<string>(),
    onStart: vi.fn(),
    onStop: vi.fn(),
    onRestart: vi.fn(),
    onOpenPort,
    onOpenDirectory: vi.fn(),
    onOpenLogs: vi.fn(),
  };
}

describe("profile browser actions", () => {
  it("disables the overview browser action for a UDP-only profile", async () => {
    const user = userEvent.setup();
    const onOpenPort = vi.fn<(profile: LaunchProfile) => void>();
    render(
      <OverviewView
        snapshot={snapshot}
        restoreBusy={false}
        onRestore={vi.fn()}
        {...commonActions(onOpenPort)}
      />,
    );

    const openButton = screen.getByRole("button", { name: "Open UDP only in browser" });
    expect(openButton).toBeDisabled();
    await user.click(openButton);
    expect(onOpenPort).not.toHaveBeenCalled();
  });

  it("disables the projects browser action for a UDP-only profile", async () => {
    const user = userEvent.setup();
    const onOpenPort = vi.fn<(profile: LaunchProfile) => void>();
    render(
      <ProjectsView
        projects={[udpProject]}
        ports={[]}
        onImport={vi.fn()}
        onAutoDiscover={vi.fn()}
        hasSavedDiscoveryRoot={false}
        monitorOnly={false}
        onEdit={vi.fn()}
        onDelete={vi.fn()}
        {...commonActions(onOpenPort)}
      />,
    );

    const openButton = screen.getByRole("button", { name: "Open UDP only in browser" });
    expect(openButton).toBeDisabled();
    await user.click(openButton);
    expect(onOpenPort).not.toHaveBeenCalled();
  });
});
