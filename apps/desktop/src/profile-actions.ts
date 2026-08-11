import type { ExpectedPort, LaunchProfile, PortSnapshot } from "./types";

export function hasTcpExpectedPort(profile: LaunchProfile): boolean {
  return profile.expectedPorts.some((port) => port.protocol === "tcp");
}

export function getOpenableProfilePort(
  profile: LaunchProfile,
  ports: PortSnapshot[],
): ExpectedPort | null {
  if (profile.status !== "running") return null;
  return profile.expectedPorts.find((expected) =>
    expected.protocol === "tcp" && ports.some((port) =>
      port.active &&
      port.protocol === "tcp" &&
      port.port === expected.port &&
      port.profileId === profile.id &&
      (port.associationSource === "managed" || port.associationSource === "confirmed"),
    ),
  ) ?? null;
}

export function canOpenProfilePort(profile: LaunchProfile, ports: PortSnapshot[]): boolean {
  return getOpenableProfilePort(profile, ports) !== null;
}
