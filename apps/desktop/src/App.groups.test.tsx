import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { api, resetMockApi } from "./api";
import App from "./App";
import type { LaunchGroupStartResult } from "./types";

/**
 * Overview's launch group band, through the real App wiring.
 *
 * The failure copy is the reason most of these exist. A whole-group start stops at
 * one member and leaves the earlier ones running, so a toast that only says "start
 * failed" tells the user neither how far it got nor which window to look at.
 */
describe("RunCove launch groups", () => {
  beforeEach(async () => {
    resetMockApi();
    window.localStorage.setItem("runcove.language", "en");
    await api.setLanguagePreference("en");
    vi.spyOn(api, "scanSavedDevelopmentRoot").mockResolvedValue([]);
  });

  afterEach(() => vi.restoreAllMocks());

  /** Render the app and wait for the group band the seeded group lives in. */
  async function openOverview() {
    const user = userEvent.setup();
    render(<App />);
    expect(await screen.findByRole("heading", { name: "Launch groups" })).toBeVisible();
    return user;
  }

  it("names the member a whole-group start stopped at", async () => {
    const start = vi.spyOn(api, "startLaunchGroup").mockResolvedValue({
      groupId: "group-studio",
      startedProfileIds: ["profile-studio-api"],
      failedProfileId: "profile-studio-web",
      error: "Expected port 5173 is occupied",
      relatedPort: { port: 5173, protocol: "tcp" },
    });
    const user = await openOverview();

    await user.click(screen.getByRole("button", { name: "Start Abyss Studio stack" }));

    const toast = await screen.findByRole("alert");
    expect(toast).toHaveTextContent(
      "Abyss Studio stack: 1 profile was started before Abyss Studio / Web failed: Expected port 5173 is occupied",
    );
    // The occupant is reachable from the failure, the same as a single-profile start.
    expect(within(toast).getByRole("button", { name: "View occupant" })).toBeVisible();
    expect(start).toHaveBeenCalledWith("group-studio");
  });

  // A group start walks its members one at a time, so a member the run has not reached
  // yet is still spoken for. Its own row buttons have to say so, or the user can race
  // the run into a conflict it was about to avoid.
  it("holds every member of a running group busy, and only that group's members", async () => {
    let settle: ((result: LaunchGroupStartResult) => void) | undefined;
    vi.spyOn(api, "startLaunchGroup").mockImplementation(
      () =>
        new Promise((resolve) => {
          settle = resolve;
        }),
    );
    const user = await openOverview();

    await user.click(screen.getByRole("button", { name: "Start Abyss Studio stack" }));

    expect(await screen.findByText("Starting...")).toBeVisible();
    expect(screen.getByRole("button", { name: "Restart Abyss Studio API" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Restart Abyss Studio Web" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Restart Docs Lab Astro" })).toBeEnabled();

    await act(async () => {
      settle?.({
        groupId: "group-studio",
        startedProfileIds: ["profile-studio-api", "profile-studio-web"],
      });
    });

    expect(await screen.findByRole("status")).toHaveTextContent(
      "Abyss Studio stack: started 2 profiles",
    );
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Restart Abyss Studio API" })).toBeEnabled(),
    );
  });

  it("refuses a restore while a group start is walking the same profiles", async () => {
    // A restore and a group start both walk profiles through the backend's per-profile
    // reservation, so overlapping them makes whichever arrives second fail with an
    // "already starting" the user did nothing to deserve. Both doors are checked: the
    // button, and the tray handler that does not go through it.
    let settle: ((result: LaunchGroupStartResult) => void) | undefined;
    vi.spyOn(api, "startLaunchGroup").mockImplementation(
      () =>
        new Promise((resolve) => {
          settle = resolve;
        }),
    );
    const restore = vi.spyOn(api, "restoreLastRunSet");
    let requestRestore: (() => void) | undefined;
    vi.spyOn(api, "onTrayRestoreRequested").mockImplementation(async (handler) => {
      requestRestore = handler;
      return () => {};
    });
    const user = await openOverview();

    await user.click(screen.getByRole("button", { name: "Start Abyss Studio stack" }));
    expect(await screen.findByText("Starting...")).toBeVisible();

    expect(screen.getByRole("button", { name: "Restore previous run" })).toBeDisabled();
    act(() => requestRestore?.());
    expect(restore).not.toHaveBeenCalled();

    await act(async () => {
      settle?.({ groupId: "group-studio", startedProfileIds: ["profile-studio-api"] });
    });

    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Restore previous run" })).toBeEnabled(),
    );
  });

  it("refuses a group start while a restore is walking the same profiles", async () => {
    let settleRestore: ((result: { startedProfileIds: string[] }) => void) | undefined;
    vi.spyOn(api, "restoreLastRunSet").mockImplementation(
      () =>
        new Promise((resolve) => {
          settleRestore = resolve;
        }),
    );
    const start = vi.spyOn(api, "startLaunchGroup");
    const user = await openOverview();

    await user.click(screen.getByRole("button", { name: "Restore previous run" }));

    const startGroup = screen.getByRole("button", { name: "Start Abyss Studio stack" });
    expect(startGroup).toBeDisabled();
    await user.click(startGroup);
    expect(start).not.toHaveBeenCalled();

    await act(async () => {
      settleRestore?.({ startedProfileIds: [] });
    });

    await waitFor(() => expect(startGroup).toBeEnabled());
  });

  it("counts the members a whole-group stop could not stop and names the first", async () => {
    vi.spyOn(api, "stopLaunchGroup").mockResolvedValue({
      groupId: "group-studio",
      stoppedProfileIds: ["profile-studio-web"],
      failures: [{ profileId: "profile-studio-api", error: "Access is denied" }],
    });
    const user = await openOverview();

    await user.click(screen.getByRole("button", { name: "Stop Abyss Studio stack" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Abyss Studio stack: 1 profile could not be stopped. Abyss Studio / API: Access is denied",
    );
  });

  it("reports how many profiles a clean whole-group stop stopped", async () => {
    vi.spyOn(api, "stopLaunchGroup").mockResolvedValue({
      groupId: "group-studio",
      stoppedProfileIds: ["profile-studio-web", "profile-studio-api"],
      failures: [],
    });
    const user = await openOverview();

    await user.click(screen.getByRole("button", { name: "Stop Abyss Studio stack" }));

    expect(await screen.findByRole("status")).toHaveTextContent(
      "Abyss Studio stack: stopped 2 profiles",
    );
  });

  // Selecting appends, so the list on the right is the sequence the user is building.
  // The arrows are the only way to change it, and a wrong index there ships a wrong
  // startup order silently.
  it("saves the startup order the editor built", async () => {
    const save = vi.spyOn(api, "saveLaunchGroup");
    const user = await openOverview();

    await user.click(screen.getByRole("button", { name: "New group" }));
    const dialog = screen.getByRole("dialog", { name: "New launch group" });
    await user.type(within(dialog).getByLabelText("Group name"), "Morning stack");
    await user.click(within(dialog).getByRole("checkbox", { name: "Astro" }));
    await user.click(within(dialog).getByRole("checkbox", { name: "API" }));
    await user.click(
      within(dialog).getByRole("button", { name: "Move Abyss Studio / API earlier" }),
    );
    await user.click(within(dialog).getByRole("button", { name: "Save group" }));

    expect(save).toHaveBeenCalledWith({
      id: undefined,
      name: "Morning stack",
      profileIds: ["profile-studio-api", "profile-docs"],
    });
    expect(await screen.findByRole("status")).toHaveTextContent("Launch group saved");
    await waitFor(() =>
      expect(screen.queryByRole("dialog", { name: "New launch group" })).toBeNull(),
    );
  });

  it("edits the group it was opened on instead of creating a second one", async () => {
    const save = vi.spyOn(api, "saveLaunchGroup");
    const user = await openOverview();

    await user.click(screen.getByRole("button", { name: "Edit Abyss Studio stack" }));
    const dialog = screen.getByRole("dialog", { name: "Edit Abyss Studio stack" });
    await user.click(within(dialog).getByRole("button", { name: "Remove Abyss Studio / Web" }));
    await user.click(within(dialog).getByRole("button", { name: "Save group" }));

    expect(save).toHaveBeenCalledWith({
      id: "group-studio",
      name: "Abyss Studio stack",
      profileIds: ["profile-studio-api"],
    });
  });

  it("refuses a duplicate group name before the round trip", async () => {
    const save = vi.spyOn(api, "saveLaunchGroup");
    const user = await openOverview();

    await user.click(screen.getByRole("button", { name: "New group" }));
    const dialog = screen.getByRole("dialog", { name: "New launch group" });
    // Lower case on purpose: the `launch_groups.name` index is `COLLATE NOCASE`, so
    // accepting this here would only move the refusal to SQLite.
    await user.type(within(dialog).getByLabelText("Group name"), "abyss studio stack");
    await user.click(within(dialog).getByRole("checkbox", { name: "Web" }));
    await user.click(within(dialog).getByRole("button", { name: "Save group" }));

    expect(within(dialog).getByText("Another launch group already uses this name.")).toBeVisible();
    expect(save).not.toHaveBeenCalled();
    expect(within(dialog).getByLabelText("Group name")).toHaveValue("abyss studio stack");
  });

  // A toast over a closed dialog would ask for the name and the order all over again,
  // so a refused save reports inline and keeps what the user built.
  it("keeps the editor open with its work when the backend refuses the save", async () => {
    vi.spyOn(api, "saveLaunchGroup").mockRejectedValue(
      new Error("A launch group with this name already exists"),
    );
    const user = await openOverview();

    await user.click(screen.getByRole("button", { name: "New group" }));
    const dialog = screen.getByRole("dialog", { name: "New launch group" });
    await user.type(within(dialog).getByLabelText("Group name"), "Morning stack");
    await user.click(within(dialog).getByRole("checkbox", { name: "Web" }));
    await user.click(within(dialog).getByRole("button", { name: "Save group" }));

    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "A launch group with this name already exists",
    );
    expect(within(dialog).getByLabelText("Group name")).toHaveValue("Morning stack");
    expect(within(dialog).getByRole("checkbox", { name: "Web" })).toBeChecked();
  });

  it("asks before deleting a group, then removes it and keeps its profiles", async () => {
    const remove = vi.spyOn(api, "deleteLaunchGroup");
    const user = await openOverview();

    await user.click(screen.getByRole("button", { name: "Delete Abyss Studio stack" }));
    const confirm = screen.getByRole("alertdialog", { name: "Delete Abyss Studio stack" });
    expect(confirm).toHaveTextContent("Its launch profiles are kept");
    await user.click(within(confirm).getByRole("button", { name: "Delete group" }));

    expect(remove).toHaveBeenCalledWith("group-studio");
    expect(await screen.findByRole("status")).toHaveTextContent("Launch group deleted");
    await waitFor(() =>
      expect(screen.queryByRole("heading", { name: "Abyss Studio stack", level: 3 })).toBeNull(),
    );
    expect(screen.getByRole("button", { name: "Start Abyss Studio API" })).toBeEnabled();
  });
});
