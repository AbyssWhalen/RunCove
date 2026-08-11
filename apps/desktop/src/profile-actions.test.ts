import { describe, expect, it } from "vitest";

import type { LaunchProfile, PortSnapshot } from "./types";
import { canOpenProfilePort, getOpenableProfilePort } from "./profile-actions";

const profile: LaunchProfile = {
  id: "profile-web",
  projectId: "project-web",
  name: "Web",
  program: "npm.cmd",
  args: ["run", "dev"],
  cwd: "D:\\projects\\web",
  expectedPorts: [{ id: "expected-web", profileId: "profile-web", port: 5173, protocol: "tcp" }],
  status: "running",
  pid: 1200,
};

const port: PortSnapshot = {
  port: 5173,
  protocol: "tcp",
  state: "LISTEN",
  bindAddress: "127.0.0.1",
  isPublic: false,
  active: true,
  pid: 1200,
  profileId: "profile-web",
  projectId: "project-web",
  associationSource: "managed",
};

describe("profile browser eligibility", () => {
  it("requires a running profile with a trusted matching active TCP port", () => {
    expect(canOpenProfilePort(profile, [port])).toBe(true);
    expect(getOpenableProfilePort(profile, [port])?.port).toBe(5173);

    expect(canOpenProfilePort({ ...profile, status: "idle" }, [port])).toBe(false);
    expect(canOpenProfilePort(profile, [{ ...port, associationSource: "suggested" }])).toBe(false);
    expect(canOpenProfilePort(profile, [{ ...port, profileId: "another-profile" }])).toBe(false);
    expect(canOpenProfilePort(profile, [{ ...port, active: false }])).toBe(false);
  });
});
