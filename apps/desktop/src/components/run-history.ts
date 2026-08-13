import type {
  LaunchProfile,
  Project,
  RunSession,
  RunSessionStatus,
} from "../types";

export type RunHistoryFilter = "all" | "active" | "exited" | "interrupted";

export interface ResolvedRunSession {
  session: RunSession;
  status: RunSessionStatus;
  project?: Project;
  profile?: LaunchProfile;
}

export function normalizeRunSessionStatus(status: unknown): RunSessionStatus {
  return status === "starting" ||
    status === "running" ||
    status === "exited" ||
    status === "interrupted"
    ? status
    : "unknown";
}

export function resolveRunSession(
  session: RunSession,
  projects: Project[],
): ResolvedRunSession {
  for (const project of projects) {
    const profile = project.profiles.find((candidate) => candidate.id === session.profileId);
    if (profile) {
      return {
        session,
        status: normalizeRunSessionStatus(session.status),
        project,
        profile,
      };
    }
  }
  return { session, status: normalizeRunSessionStatus(session.status) };
}

export function prepareRunHistory(
  sessions: RunSession[],
  projects: Project[],
  query = "",
  filter: RunHistoryFilter = "all",
): ResolvedRunSession[] {
  const normalizedQuery = query.trim().toLocaleLowerCase();
  return sessions
    .map((session) => resolveRunSession(session, projects))
    .sort((left, right) => right.session.startedAt - left.session.startedAt)
    .filter((entry) => {
      if (filter === "active" && entry.status !== "starting" && entry.status !== "running") {
        return false;
      }
      if (filter === "exited" && entry.status !== "exited") return false;
      if (filter === "interrupted" && entry.status !== "interrupted") return false;
      if (!normalizedQuery) return true;
      return [entry.project?.name, entry.profile?.name, entry.session.profileName]
        .some((value) => value?.toLocaleLowerCase().includes(normalizedQuery));
    });
}

export function formatRunDuration(
  session: Pick<RunSession, "startedAt" | "endedAt">,
  locale: string,
  now = Date.now(),
): string {
  const durationMs = Math.max(0, (session.endedAt ?? now) - session.startedAt);
  const totalSeconds = Math.floor(durationMs / 1_000);
  const days = Math.floor(totalSeconds / 86_400);
  const hours = Math.floor((totalSeconds % 86_400) / 3_600);
  const minutes = Math.floor((totalSeconds % 3_600) / 60);
  const seconds = totalSeconds % 60;
  const values = days > 0
    ? [[days, "day"], [hours, "hour"]] as const
    : hours > 0
      ? [[hours, "hour"], [minutes, "minute"]] as const
      : minutes > 0
        ? [[minutes, "minute"], [seconds, "second"]] as const
        : [[seconds, "second"]] as const;
  return values
    .filter(([value], index) => value > 0 || index === 0)
    .map(([value, unit]) => new Intl.NumberFormat(locale, {
      style: "unit",
      unit,
      unitDisplay: "narrow",
    }).format(value))
    .join(" ");
}
