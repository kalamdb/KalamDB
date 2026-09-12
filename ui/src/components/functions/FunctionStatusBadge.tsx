import { Badge } from "@/components/ui/badge";
import { statusLabel } from "@/features/functions/display";
import type { ProcedureStatus } from "@/features/functions/types";
import { cn } from "@/lib/utils";

const DOT_CLASS: Record<ProcedureStatus, string> = {
  ready: "bg-emerald-500",
  warning: "bg-amber-500",
  error: "bg-destructive",
};

export function FunctionStatusBadge({ status }: { status: ProcedureStatus }) {
  return (
    <Badge variant="outline" className="gap-1.5 font-normal">
      <span className={cn("size-1.5 rounded-full", DOT_CLASS[status])} />
      {statusLabel(status)}
    </Badge>
  );
}
