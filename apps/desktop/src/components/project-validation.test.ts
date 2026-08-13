import { describe, expect, it } from "vitest";

import type { ProjectInput } from "../types";
import { copyLaunchProfile, hasProjectValidationErrors, validateProjectInput } from "./project-validation";

function validInput(): ProjectInput {
  return {
    name: "Web App",
    path: "D:\\projects\\web-app",
    profiles: [{
      name: "dev",
      program: "npm.cmd",
      args: ["run", "dev"],
      cwd: "D:\\projects\\web-app",
      expectedPorts: [{ port: 3000, protocol: "tcp" }],
    }],
  };
}

describe("project validation", () => {
  it("copies only editable launch-profile fields", () => {
    const copied = copyLaunchProfile({
      id: "profile-id",
      name: "dev",
      program: "npm.cmd",
      args: ["run", "dev"],
      cwd: "D:\\projects\\web-app",
      observedRuntime: true,
      expectedPorts: [{ id: "port-id", port: 3000, protocol: "tcp" }],
    }, "Copy");

    expect(copied).toEqual({
      name: "dev Copy",
      program: "npm.cmd",
      args: ["run", "dev"],
      cwd: "D:\\projects\\web-app",
      expectedPorts: [{ port: 3000, protocol: "tcp" }],
    });
  });

  it("accepts a complete structured launch configuration", () => {
    expect(hasProjectValidationErrors(validateProjectInput(validInput()))).toBe(false);
  });

  it("reports required fields, invalid ports, and every duplicate port pair", () => {
    const input = validInput();
    input.name = " ";
    input.path = "";
    input.profiles[0].name = "";
    input.profiles[0].program = " ";
    input.profiles[0].cwd = "";
    input.profiles[0].args.push("");
    input.profiles[0].expectedPorts = [
      { port: 0, protocol: "tcp" },
      { port: 3000, protocol: "udp" },
      { port: 3000, protocol: "udp" },
    ];

    const errors = validateProjectInput(input);

    expect(errors.name).toBe("required");
    expect(errors.path).toBe("required");
    expect(errors.profileErrors[0]).toMatchObject({
      name: "required",
      program: "required",
      cwd: "required",
      args: [undefined, undefined, "required"],
      ports: ["range", "duplicate", "duplicate"],
    });
    expect(hasProjectValidationErrors(errors)).toBe(true);
  });
});
