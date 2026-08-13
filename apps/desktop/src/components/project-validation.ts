import type { LaunchProfileInput, ProjectInput } from "../types";

export type FieldError = "required";
export type PortError = "duplicate" | "range";

export interface ProfileValidationErrors {
  name?: FieldError;
  program?: FieldError;
  cwd?: FieldError;
  args: Array<FieldError | undefined>;
  ports: Array<PortError | undefined>;
}

export interface ProjectValidationErrors {
  name?: FieldError;
  path?: FieldError;
  profiles?: FieldError;
  profileErrors: ProfileValidationErrors[];
}

export function copyLaunchProfile(profile: LaunchProfileInput, nameSuffix: string): LaunchProfileInput {
  return {
    name: `${profile.name} ${nameSuffix}`.trim(),
    program: profile.program,
    args: [...profile.args],
    cwd: profile.cwd,
    expectedPorts: profile.expectedPorts.map(({ port, protocol }) => ({ port, protocol })),
  };
}

export function validateProjectInput(input: ProjectInput): ProjectValidationErrors {
  return {
    name: input.name.trim() ? undefined : "required",
    path: input.path.trim() ? undefined : "required",
    profiles: input.profiles.length > 0 ? undefined : "required",
    profileErrors: input.profiles.map((profile) => {
      const portCounts = new Map<string, number>();
      for (const expectedPort of profile.expectedPorts) {
        const key = `${expectedPort.protocol}:${expectedPort.port}`;
        portCounts.set(key, (portCounts.get(key) ?? 0) + 1);
      }

      return {
        name: profile.name.trim() ? undefined : "required",
        program: profile.program.trim() ? undefined : "required",
        cwd: profile.cwd.trim() ? undefined : "required",
        args: profile.args.map((argument) => argument.trim() ? undefined : "required"),
        ports: profile.expectedPorts.map((expectedPort) => {
          if (!Number.isInteger(expectedPort.port) || expectedPort.port < 1 || expectedPort.port > 65_535) {
            return "range";
          }
          return (portCounts.get(`${expectedPort.protocol}:${expectedPort.port}`) ?? 0) > 1
            ? "duplicate"
            : undefined;
        }),
      };
    }),
  };
}

export function hasProjectValidationErrors(errors: ProjectValidationErrors): boolean {
  return Boolean(
    errors.name ||
    errors.path ||
    errors.profiles ||
    errors.profileErrors.some((profile) =>
      profile.name ||
      profile.program ||
      profile.cwd ||
      profile.args.some(Boolean) ||
      profile.ports.some(Boolean),
    ),
  );
}
