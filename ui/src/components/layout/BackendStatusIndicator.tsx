import { Loader2, Wifi, WifiOff } from "lucide-react";
import { useBackendStatus } from "@/lib/backend-status";
import { cn } from "@/lib/utils";

function formatTarget(origin: string): string {
  try {
    return new URL(origin).host;
  } catch {
    return origin;
  }
}

export default function BackendStatusIndicator() {
  const { state, message, targetOrigin, usingWebSocket } = useBackendStatus();
  const targetLabel = formatTarget(targetOrigin);
  const isOnline = state === "online";
  const isChecking = state === "checking";
  const statusLabel = isOnline ? "Online" : isChecking ? "Connecting" : "Offline";
  const transportLabel = usingWebSocket ? "WS" : "API";

  return (
    <div
      className="flex items-center gap-2 rounded-md border border-border bg-muted/40 px-2.5 py-1 text-xs text-foreground"
      title={`${transportLabel} ${statusLabel} · ${targetLabel} · ${message}`}
      aria-label={`${transportLabel} ${statusLabel}`}
    >
      <span
        className={cn(
          "h-2 w-2 rounded-full shrink-0",
          isOnline && "bg-emerald-500",
          isChecking && "bg-amber-400",
          state === "offline" && "bg-destructive",
        )}
      />
      <span className="font-medium">{transportLabel}</span>
      <span className="hidden text-muted-foreground md:inline">{statusLabel}</span>
      <span className="hidden max-w-[160px] truncate text-muted-foreground xl:inline">{targetLabel}</span>
      {isChecking ? (
        <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-muted-foreground" />
      ) : isOnline ? (
        <Wifi className="h-3.5 w-3.5 shrink-0 text-emerald-600" />
      ) : (
        <WifiOff className="h-3.5 w-3.5 shrink-0 text-destructive" />
      )}
    </div>
  );
}