import { describe, expect, it, vi } from "vitest";

import { createMockApi } from "./mock-data";

describe("browser mock API", () => {
  it("keeps project actions and bounded logs deterministic", async () => {
    const mock = createMockApi();
    const onStatus = vi.fn();
    const onLog = vi.fn();
    const stopStatus = await mock.onRunStatus(onStatus);
    const stopLog = await mock.onRunLog(onLog);

    const status = await mock.startProfile("profile-studio-api");
    const snapshot = await mock.getDashboardSnapshot();
    const profile = snapshot.projects
      .flatMap((project) => project.profiles)
      .find((item) => item.id === "profile-studio-api");

    expect(status.status).toBe("running");
    expect(profile?.status).toBe("running");
    expect(onStatus).toHaveBeenCalledWith(expect.objectContaining({ profileId: "profile-studio-api" }));
    expect(onLog).toHaveBeenCalledWith(expect.objectContaining({ line: "Start requested from browser preview" }));
    expect(await mock.getLogs("profile-studio-api")).toHaveLength(1);

    stopStatus();
    stopLog();
  });

  it("rechecks external process identity inputs before removing the mock listener", async () => {
    const mock = createMockApi();
    const before = await mock.getDashboardSnapshot();
    const external = before.ports.find((port) => port.pid === 9208);
    expect(external?.active).toBe(true);
    if (
      !external ||
      external.pid == null ||
      !external.processStartedAt ||
      !external.executablePath
    ) {
      throw new Error("expected the external mock listener to expose a complete process identity");
    }

    await mock.terminateExternalProcess({
      port: external.port,
      protocol: external.protocol,
      pid: external.pid,
      startedAt: external.processStartedAt,
      executablePath: external.executablePath,
    });

    const after = await mock.getDashboardSnapshot();
    expect(after.ports.find((port) => port.port === 4000)).toMatchObject({ active: false, pid: null });
  });

  it("matches the lowercase protocol and epoch-millisecond wire contract", async () => {
    const mock = createMockApi();
    const snapshot = await mock.getDashboardSnapshot();

    expect(typeof snapshot.generatedAt).toBe("number");
    expect(typeof snapshot.projects[0]?.createdAt).toBe("number");
    expect(typeof snapshot.ports[0]?.processStartedAt).toBe("number");
    expect(snapshot.ports.every((port) => port.protocol === "tcp" || port.protocol === "udp")).toBe(true);
    expect(snapshot.settings).toEqual(expect.objectContaining({ pollIntervalMs: 2_000, logCapacity: 500, closeBehavior: "ask" }));
  });

  it("persists and resets the title-bar close behavior in the settings contract", async () => {
    const mock = createMockApi();

    await mock.setCloseBehavior("hideToTray");
    expect((await mock.getDashboardSnapshot()).settings.closeBehavior).toBe("hideToTray");

    await mock.setCloseBehavior("ask");
    expect((await mock.getDashboardSnapshot()).settings.closeBehavior).toBe("ask");
  });

  it("remembers one development root and returns candidates without importing them", async () => {
    const mock = createMockApi();
    const root = "D:\\work\\dev";

    const firstCandidates = await mock.scanDevelopmentRoot(`${root}\\`);
    const afterFirstScan = await mock.getDashboardSnapshot();
    expect(afterFirstScan.settings.recentDevelopmentRoot).toBe(root);
    expect(afterFirstScan.projects.some((project) => project.path.startsWith(root))).toBe(false);

    const savedCandidates = await mock.scanSavedDevelopmentRoot();
    expect(savedCandidates).toEqual(firstCandidates);
    const afterSavedScan = await mock.getDashboardSnapshot();
    expect(afterSavedScan.projects).toHaveLength(afterFirstScan.projects.length);
  });
});
