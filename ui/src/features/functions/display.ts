import { formatRelativeTime, formatUtcTimestamp } from "@/lib/formatters";
import { displayLogLevel } from "./format";
import type { ProcedureStatus } from "./types";

export function rtkErrorMessage(error: unknown, fallback: string): string | null {
  if (!error) {
    return null;
  }
  if (typeof error === "object" && error !== null && "error" in error) {
    const message = (error as { error?: unknown }).error;
    if (typeof message === "string" && message.trim()) {
      return message;
    }
  }
  return fallback;
}

export function statusLabel(status: ProcedureStatus): string {
  switch (status) {
    case "ready":
      return "Ready";
    case "warning":
      return "Warning";
    case "error":
      return "Error";
  }
}

export function formatCount(value: number | null | undefined): string {
  if (value === null || value === undefined || !Number.isFinite(value)) {
    return "—";
  }
  return value.toLocaleString();
}

export function formatEpochMs(value: number | null | undefined): string {
  if (value === null || value === undefined || !Number.isFinite(value) || value <= 0) {
    return "—";
  }
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return "—";
  }
  const ageMs = Date.now() - date.getTime();
  if (ageMs >= 0 && ageMs < 7 * 24 * 60 * 60 * 1000) {
    return formatRelativeTime(date);
  }
  return formatUtcTimestamp(value);
}

export function formatLogTime(timestamp: string): string {
  const parsed = Date.parse(timestamp);
  if (Number.isNaN(parsed)) {
    return timestamp || "—";
  }
  return new Date(parsed).toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  });
}

export function formatLogTimestamp(timestamp: string): string {
  const parsed = Date.parse(timestamp);
  if (Number.isNaN(parsed)) {
    return timestamp || "—";
  }
  return formatUtcTimestamp(parsed);
}

export function logLevelClassName(level: string): string {
  const normalized = displayLogLevel(level);
  if (normalized === "ERROR") {
    return "text-destructive";
  }
  if (normalized === "WARN") {
    return "text-amber-600 dark:text-amber-400";
  }
  return "text-muted-foreground";
}
