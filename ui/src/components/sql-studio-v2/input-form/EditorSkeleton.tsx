import { Loader2 } from "lucide-react";

export function EditorSkeleton() {
  return (
    <div className="flex h-full min-h-0 flex-col overflow-hidden bg-background">
      <div className="flex shrink-0 items-center justify-between border-b border-border px-4 py-2">
        <div className="space-y-1.5">
          <div className="h-4 w-40 animate-pulse rounded bg-muted/60" />
          <div className="h-3 w-14 animate-pulse rounded bg-muted/40" />
        </div>
        <div className="h-7 w-20 animate-pulse rounded bg-muted/40" />
      </div>
      <div className="flex flex-1 items-center justify-center text-xs text-muted-foreground">
        <Loader2 className="mr-2 h-3.5 w-3.5 animate-spin" />
        Loading editor…
      </div>
    </div>
  );
}
