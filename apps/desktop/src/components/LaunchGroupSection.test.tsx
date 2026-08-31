import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { I18nProvider } from "../i18n";
import type { LaunchGroup, LaunchProfile, ProfileStatus, Project } from "../types";
import { LaunchGroupSection, type GroupAction } from "./LaunchGroupSection";

function profile(id: string, name: string, status: ProfileStatus): LaunchProfile {
  return {
    id,
    projectId: "project-studio",
    name,
    program: "npm.cmd",
    args: ["run", id],
    cwd: "D:\\projects\\studio",
    expectedPorts: [],
    status,
    pid: status === "running" ? 4321 : null,
  };
}

const projects: Project[] = [
  {
    id: "project-studio",
    name: "Abyss Studio",
    path: "D:\\projects\\studio",
    profiles: [
      profile("db", "Database", "running"),
      profile("api", "API", "running"),
      profile("web", "Web", "idle"),
    ],
    createdAt: 1,
    updatedAt: 1,
  },
  {
    id: "project-docs",
    name: "Docs Lab",
    path: "D:\\projects\\docs",
    profiles: [{ ...profile("astro", "Astro", "idle"), projectId: "project-docs" }],
    createdAt: 1,
    updatedAt: 1,
  },
];

function group(id: string, name: string, profileIds: string[]): LaunchGroup {
  return { id, name, profileIds, createdAt: 1, updatedAt: 2 };
}

const mixed = group("group-mixed", "Morning stack", ["db", "web"]);
const allUp = group("group-up", "Everything up", ["db", "api"]);
const allDown = group("group-down", "Everything down", ["web", "astro"]);

type SectionProps = React.ComponentProps<typeof LaunchGroupSection>;

function renderSection(overrides: Partial<SectionProps> = {}) {
  const handlers = {
    onNew: vi.fn<() => void>(),
    onEdit: vi.fn<(group: LaunchGroup) => void>(),
    onDelete: vi.fn<(group: LaunchGroup) => void>(),
    onStart: vi.fn<(group: LaunchGroup) => void>(),
    onStop: vi.fn<(group: LaunchGroup) => void>(),
  };
  const props: SectionProps = {
    groups: [mixed, allUp, allDown],
    projects,
    monitorOnly: false,
    busyGroups: new Map<string, GroupAction>(),
    ...handlers,
    ...overrides,
  };
  render(
    <I18nProvider>
      <LaunchGroupSection {...props} />
    </I18nProvider>,
  );
  return handlers;
}

/** The one card a group's name belongs to, so a sibling group cannot answer for it. */
function card(name: string): HTMLElement {
  const heading = screen.getByRole("heading", { name, level: 3 });
  const element = heading.closest("li");
  if (!element) throw new Error(`no launch group card for ${name}`);
  return element;
}

describe("LaunchGroupSection", () => {
  it("hands each action the group whose row was pressed", async () => {
    const user = userEvent.setup();
    const handlers = renderSection();

    await user.click(screen.getByRole("button", { name: "Start Everything down" }));
    await user.click(screen.getByRole("button", { name: "Stop Everything up" }));
    await user.click(screen.getByRole("button", { name: "Edit Morning stack" }));
    await user.click(screen.getByRole("button", { name: "Delete Morning stack" }));
    await user.click(screen.getByRole("button", { name: "New group" }));

    expect(handlers.onStart).toHaveBeenCalledOnce();
    expect(handlers.onStart).toHaveBeenCalledWith(allDown);
    expect(handlers.onStop).toHaveBeenCalledOnce();
    expect(handlers.onStop).toHaveBeenCalledWith(allUp);
    expect(handlers.onEdit).toHaveBeenCalledOnce();
    expect(handlers.onEdit).toHaveBeenCalledWith(mixed);
    expect(handlers.onDelete).toHaveBeenCalledOnce();
    expect(handlers.onDelete).toHaveBeenCalledWith(mixed);
    expect(handlers.onNew).toHaveBeenCalledOnce();
  });

  it("lists members in launch order and badges how much of the group is up", () => {
    renderSection();

    const order = within(card("Morning stack")).getByRole("list", { name: "Startup order" });
    expect(within(order).getAllByRole("listitem").map((item) => item.textContent)).toEqual([
      "Abyss Studio / Database",
      "Abyss Studio / Web",
    ]);
    expect(within(card("Morning stack")).getByText("Partly running")).toBeInTheDocument();
    expect(within(card("Everything up")).getByText("All running")).toBeInTheDocument();
    expect(within(card("Everything down")).getByText("Not running")).toBeInTheDocument();
  });

  it("refuses to start a group already up and to stop one already down", () => {
    renderSection();

    expect(screen.getByRole("button", { name: "Start Everything up" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Stop Everything up" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Start Everything down" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Stop Everything down" })).toBeDisabled();
  });

  it("locks a group's whole row while one of its actions is in flight", () => {
    renderSection({ busyGroups: new Map([[mixed.id, "start"]]) });

    const busy = card("Morning stack");
    expect(within(busy).getByText("Starting...")).toBeInTheDocument();
    expect(within(busy).getByRole("button", { name: "Start Morning stack" })).toBeDisabled();
    expect(within(busy).getByRole("button", { name: "Stop Morning stack" })).toBeDisabled();
    // Editing or deleting a group mid-start would change what the run is walking.
    expect(within(busy).getByRole("button", { name: "Edit Morning stack" })).toBeDisabled();
    expect(within(busy).getByRole("button", { name: "Delete Morning stack" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Start Everything down" })).toBeEnabled();
  });

  it("says a stop is the pending action rather than a shared 'working'", () => {
    renderSection({ busyGroups: new Map([[allUp.id, "stop"]]) });

    expect(within(card("Everything up")).getByText("Stopping...")).toBeInTheDocument();
    expect(within(card("Morning stack")).getByText("Start")).toBeInTheDocument();
  });

  it("blocks the process actions in monitor-only mode but still allows editing", () => {
    renderSection({ monitorOnly: true });

    const suffix = ": Unavailable in administrator monitor-only mode";
    expect(screen.getByRole("button", { name: `Start Everything down${suffix}` })).toBeDisabled();
    expect(screen.getByRole("button", { name: `Stop Everything up${suffix}` })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Edit Morning stack" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Delete Morning stack" })).toBeEnabled();
  });

  it("keeps a group emptied by the profile cascade visible and unusable", () => {
    renderSection({ groups: [group("group-empty", "Emptied", [])] });

    expect(
      screen.getByText("Every profile in this group was deleted. Edit the group to add profiles."),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Start Emptied" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Stop Emptied" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Edit Emptied" })).toBeEnabled();
  });

  it("names a member this build cannot resolve instead of shortening the list", () => {
    renderSection({ groups: [group("group-ghost", "Half known", ["db", "profile-gone"])] });

    const order = screen.getByRole("list", { name: "Startup order" });
    expect(within(order).getAllByRole("listitem").map((item) => item.textContent)).toEqual([
      "Abyss Studio / Database",
      "profile-gone - profile not found",
    ]);
    expect(screen.getByText("Partly running")).toBeInTheDocument();
  });

  it("offers the editor when there are no groups at all", () => {
    renderSection({ groups: [] });

    expect(
      screen.getByText("No launch groups yet. Create one to bring up a whole stack at once."),
    ).toBeInTheDocument();
    expect(screen.getByText("0 groups · Start or stop a whole stack in one step, in the order you set."))
      .toBeInTheDocument();
    expect(screen.getByRole("button", { name: "New group" })).toBeEnabled();
  });
});
