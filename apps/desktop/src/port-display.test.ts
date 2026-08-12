import { describe, expect, it } from "vitest";

import { activeDisplayPortCount, groupDisplayPorts } from "./port-display";
import type { PortSnapshot } from "./types";

function port(overrides: Partial<PortSnapshot> = {}): PortSnapshot {
  return {
    port: 4_321,
    protocol: "tcp",
    state: "LISTEN",
    bindAddress: "0.0.0.0",
    isPublic: true,
    active: true,
    pid: 123,
    processName: "node.exe",
    projectId: null,
    profileId: null,
    associationSource: null,
    ...overrides,
  };
}

describe("port display grouping", () => {
  it("merges duplicate and dual-stack rows for the same process", () => {
    const grouped = groupDisplayPorts([
      port(),
      port(),
      port({ bindAddress: "::" }),
    ]);

    expect(grouped).toHaveLength(1);
    expect(grouped[0].bindAddress).toBe("0.0.0.0, ::");
    expect(activeDisplayPortCount([port(), port({ bindAddress: "::" })])).toBe(1);
  });

  it("keeps different processes and historical ownership separate", () => {
    const grouped = groupDisplayPorts([
      port({ pid: 123 }),
      port({ pid: 456 }),
      port({ active: false, pid: null, bindAddress: null, projectId: "history-a" }),
      port({ active: false, pid: null, bindAddress: null, projectId: "history-b" }),
    ]);

    expect(grouped).toHaveLength(4);
  });
});
