import type { PortSnapshot } from "./types";

function activePortKey(port: PortSnapshot): string | null {
  if (!port.active || port.pid == null) return null;
  return [
    port.protocol,
    port.port,
    port.pid,
    port.projectId ?? "",
    port.profileId ?? "",
    port.associationSource ?? "",
  ].join(":");
}

function mergedAddresses(left?: string | null, right?: string | null): string | null {
  const addresses = [...new Set([left, right].filter((value): value is string => Boolean(value)))];
  return addresses.length > 0 ? addresses.join(", ") : null;
}

export function groupDisplayPorts(ports: PortSnapshot[]): PortSnapshot[] {
  const grouped: PortSnapshot[] = [];
  const activeIndexes = new Map<string, number>();

  for (const port of ports) {
    const key = activePortKey(port);
    const existingIndex = key == null ? undefined : activeIndexes.get(key);
    if (existingIndex == null) {
      if (key != null) activeIndexes.set(key, grouped.length);
      grouped.push({ ...port });
      continue;
    }

    const existing = grouped[existingIndex];
    grouped[existingIndex] = {
      ...existing,
      bindAddress: mergedAddresses(existing.bindAddress, port.bindAddress),
      isPublic: existing.isPublic || port.isPublic,
      lastSeenAt: Math.max(existing.lastSeenAt ?? 0, port.lastSeenAt ?? 0) || null,
    };
  }

  return grouped;
}

export function activeDisplayPortCount(ports: PortSnapshot[]): number {
  return groupDisplayPorts(ports).filter((port) => port.active).length;
}
