import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";

export function EditorSkeleton() {
  return (
    <div className="flex h-full min-h-0 flex-col overflow-hidden bg-background">
      <div className="flex shrink-0 items-center justify-between border-b border-border px-4 py-2">
        <div className="space-y-1.5">
          <Skeleton className="h-4 w-40" />
          <Skeleton className="h-3 w-14" />
        </div>
        <Skeleton className="h-7 w-20" />
      </div>
      <div className="flex flex-1 items-center justify-center gap-2 text-xs text-muted-foreground">
        <Spinner className="size-3.5" />
        Loading editor…
      </div>
    </div>
  );
}
