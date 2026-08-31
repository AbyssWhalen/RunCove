import type { LaunchGroup, LaunchProfile } from "../types";

/** What a launch group looks like to the surfaces that render it. */
export type LaunchGroupStatus = "running" | "partial" | "idle";

/**
 * The two profile fields a group cares about, plus the id it is listed under.
 *
 * Narrow on purpose: the section resolves members from the dashboard snapshot, but
 * a test should be able to state a member in three fields rather than build a whole
 * `LaunchProfile`.
 */
type MemberProfile = Pick<LaunchProfile, "id" | "name" | "status">;

/** A project as a member lookup sees it: its name, and the profiles under it. */
export interface MemberProject {
  name: string;
  profiles: MemberProfile[];
}

/**
 * One member of a group, resolved against the projects this build can see.
 *
 * `missing` is not a defect to be hidden. A profile deleted while it was in a group
 * is cascaded out by the database, so a missing member here means the snapshot the
 * group came from is older than the one the projects came from — a state that lasts
 * until the next poll. Showing the raw id is what makes that legible instead of
 * silently shortening the list.
 */
export interface LaunchGroupMember {
  profileId: string;
  /** `Project / Profile`, or the raw id when this build cannot find the profile. */
  label: string;
  /** `null` for a member no project claims. */
  status: LaunchProfile["status"] | null;
  missing: boolean;
}

/**
 * The statuses that mean "RunCove is holding this profile up".
 *
 * Same reading as the profile rows use (`OverviewView.tsx`, `ProjectsView.tsx`):
 * `starting` counts as up because stopping is the action that applies to it, and
 * `conflict` does not, because the port belongs to something RunCove did not start.
 */
function isUp(status: LaunchProfile["status"] | null): boolean {
  return status === "running" || status === "starting";
}

/**
 * Resolve a group's members in launch order.
 *
 * Order comes from `profileIds` and is never sorted here: it is the order the
 * backend starts them in, so re-ordering it for display would misreport the
 * feature.
 */
export function resolveLaunchGroupMembers(
  group: Pick<LaunchGroup, "profileIds">,
  projects: MemberProject[],
): LaunchGroupMember[] {
  const lookup = new Map<string, { project: MemberProject; profile: MemberProfile }>();
  for (const project of projects) {
    for (const profile of project.profiles) {
      lookup.set(profile.id, { project, profile });
    }
  }
  return group.profileIds.map((profileId) => {
    const entry = lookup.get(profileId);
    if (!entry) {
      return { profileId, label: profileId, status: null, missing: true };
    }
    return {
      profileId,
      label: `${entry.project.name} / ${entry.profile.name}`,
      status: entry.profile.status,
      missing: false,
    };
  });
}

/**
 * Which of the three badges a group wears.
 *
 * An empty group is `idle` rather than `running`: an `every` over nothing is
 * vacuously true, and a group whose members were all deleted must not claim to be
 * up. A group with only missing members is `idle` for the same reason.
 */
export function deriveLaunchGroupStatus(members: LaunchGroupMember[]): LaunchGroupStatus {
  if (members.length === 0) return "idle";
  const up = members.filter((member) => isUp(member.status)).length;
  if (up === 0) return "idle";
  return up === members.length ? "running" : "partial";
}

export type LaunchGroupFieldError = "required" | "duplicate";

export interface LaunchGroupValidationErrors {
  /** Set when the name is blank once trimmed, or collides with another group's. */
  name?: LaunchGroupFieldError;
  /** Set when nothing is selected. An empty group's start button means nothing. */
  members?: "required";
}

/**
 * What the editor refuses to save.
 *
 * These mirror the checks `Storage::validate_launch_group` makes, so the dialog can
 * say no before a round trip. The backend still checks: this is the faster answer,
 * not the authoritative one. Duplicate member ids are not checked because the
 * editor's selection is a set — it cannot express one.
 */
export function validateLaunchGroupInput(
  input: { id?: string; name: string; profileIds: string[] },
  existingGroups: Array<Pick<LaunchGroup, "id" | "name">>,
): LaunchGroupValidationErrors {
  const errors: LaunchGroupValidationErrors = {};
  const name = input.name.trim();
  if (!name) {
    errors.name = "required";
  } else if (
    existingGroups.some(
      (group) => group.id !== input.id && group.name.toLowerCase() === name.toLowerCase(),
    )
  ) {
    // Case-insensitive because the `launch_groups.name` index is `COLLATE NOCASE`:
    // accepting "Stack" next to "stack" here would only move the refusal to SQLite.
    errors.name = "duplicate";
  }
  if (input.profileIds.length === 0) errors.members = "required";
  return errors;
}

export function hasLaunchGroupValidationErrors(errors: LaunchGroupValidationErrors): boolean {
  return Boolean(errors.name ?? errors.members);
}
