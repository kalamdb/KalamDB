import type { FunctionDetailTab } from "./types";

export function functionListPath(): string {
  return "/functions";
}

export function functionDetailPath(procedureId: string, tab: FunctionDetailTab = "overview"): string {
  const encoded = encodeURIComponent(procedureId);
  if (tab === "overview") {
    return `/functions/${encoded}`;
  }
  return `/functions/${encoded}/${tab}`;
}
