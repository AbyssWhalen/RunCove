import { describe, expect, it } from "vitest";

import type { LaunchProfile } from "../types";
import {
  deriveLaunchGroupStatus,
  hasLaunchGroupValidationErrors,
  resolveLaunchGroupMembers,
  validateLaunchGroupInput,
  type LaunchGroupMember,
  type LaunchGroupStatus,
  type MemberProject,
} from "./launch-group";

function projects(): MemberProject[] {
  return [
    {
      name: "Abyss Studio",
      profiles: [
        { id: "web", name: "Web", status: "running" },
        { id: "api", name: "API", status: "idle" },
      ],
    },
    { name: "Docs Lab", profiles: [{ id: "docs", name: "Astro", status: "starting" }] },
  ];
}

function membersWith(statuses: Array<LaunchProfile["status"] | null>): LaunchGroupMember[] {
  return statuses.map((status, index) => ({
    profileId: `profile-${index}`,
    label: `Project / Profile ${index}`,
    status,
    missing: status === null,
  }));
}

describe("deriveLaunchGroupStatus", () => {
  const cases: Array<{
    label: string;
    statuses: Array<LaunchProfile["status"] | null>;
    expected: LaunchGroupStatus;
  }> = [
    { label: "every member is up", statuses: ["running", "starting"], expected: "running" },
    { label: "one of two members is up", statuses: ["running", "idle"], expected: "partial" },
    { label: "no member is up", statuses: ["idle", "exited"], expected: "idle" },
    // A conflict is a port RunCove did not open, so a group of conflicts is not up.
    { label: "every member is in conflict", statuses: ["conflict", "conflict"], expected: "idle" },
    { label: "a conflict sits beside a running member", statuses: ["running", "conflict"], expected: "partial" },
    { label: "a member is missing", statuses: ["running", null], expected: "partial" },
    // An `every` over nothing is vacuously true; an emptied group must not read "up".
    { label: "the group has no members", statuses: [], expected: "idle" },
  ];

  it.each(cases)("reads $expected when $label", ({ statuses, expected }) => {
    expect(deriveLaunchGroupStatus(membersWith(statuses))).toBe(expected);
  });
});

describe("resolveLaunchGroupMembers", () => {
  it("keeps launch order and labels each member with its project", () => {
    expect(resolveLaunchGroupMembers({ profileIds: ["docs", "api", "web"] }, projects())).toEqual([
      { profileId: "docs", label: "Docs Lab / Astro", status: "starting", missing: false },
      { profileId: "api", label: "Abyss Studio / API", status: "idle", missing: false },
      { profileId: "web", label: "Abyss Studio / Web", status: "running", missing: false },
    ]);
  });

  it("marks a member no project claims instead of dropping it", () => {
    expect(resolveLaunchGroupMembers({ profileIds: ["web", "ghost"] }, projects())).toEqual([
      { profileId: "web", label: "Abyss Studio / Web", status: "running", missing: false },
      { profileId: "ghost", label: "ghost", status: null, missing: true },
    ]);
  });
});

describe("validateLaunchGroupInput", () => {
  const existing = [
    { id: "group-stack", name: "Full stack" },
    { id: "group-docs", name: "Docs" },
  ];

  it.each([
    {
      label: "a name that is only whitespace",
      input: { name: "   ", profileIds: ["web"] },
      expected: { name: "required" },
    },
    {
      label: "a name another group already uses",
      input: { name: "Docs", profileIds: ["web"] },
      expected: { name: "duplicate" },
    },
    {
      label: "a name that differs from another group's only in case",
      input: { name: "fUlL sTaCk", profileIds: ["web"] },
      expected: { name: "duplicate" },
    },
    {
      label: "a name that differs only by surrounding whitespace",
      input: { name: "  Docs  ", profileIds: ["web"] },
      expected: { name: "duplicate" },
    },
    {
      label: "nothing selected",
      input: { name: "Morning", profileIds: [] },
      expected: { members: "required" },
    },
    {
      label: "both an empty name and nothing selected",
      input: { name: "", profileIds: [] },
      expected: { name: "required", members: "required" },
    },
  ])("refuses $label", ({ input, expected }) => {
    const errors = validateLaunchGroupInput(input, existing);
    expect(errors).toEqual(expected);
    expect(hasLaunchGroupValidationErrors(errors)).toBe(true);
  });

  it("lets a group keep its own name while it is edited", () => {
    const errors = validateLaunchGroupInput(
      { id: "group-docs", name: "Docs", profileIds: ["docs"] },
      existing,
    );
    expect(errors).toEqual({});
    expect(hasLaunchGroupValidationErrors(errors)).toBe(false);
  });

  it("accepts a new group with a free name and one member", () => {
    expect(
      validateLaunchGroupInput({ name: "Morning stack", profileIds: ["web", "api"] }, existing),
    ).toEqual({});
  });
});
