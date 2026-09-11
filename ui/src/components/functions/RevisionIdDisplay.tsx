import { useState } from "react";
import { Check, Copy } from "lucide-react";
import { Button } from "@/components/ui/button";
import { shortenRevisionId } from "@/features/functions/format";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";

export function RevisionIdDisplay({
  revisionId,
  empty = "—",
}: {
  revisionId: string | null | undefined;
  empty?: string;
}) {
  const [copied, setCopied] = useState(false);

  if (!revisionId) {
    return <span className="text-muted-foreground">{empty}</span>;
  }

  const short = shortenRevisionId(revisionId);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(revisionId);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch {
      setCopied(false);
    }
  };

  return (
    <TooltipProvider delayDuration={200}>
      <span className="inline-flex max-w-full items-center gap-1 font-mono text-xs">
        <Tooltip>
          <TooltipTrigger asChild>
            <span className="truncate" title={revisionId}>
              {short}
            </span>
          </TooltipTrigger>
          {short !== revisionId && (
            <TooltipContent side="top" className="max-w-sm break-all font-mono text-[11px]">
              {revisionId}
            </TooltipContent>
          )}
        </Tooltip>
        <Button
          type="button"
          variant="ghost"
          size="icon-xxs"
          className="text-muted-foreground"
          onClick={(event) => {
            event.stopPropagation();
            void copy();
          }}
          aria-label="Copy revision ID"
        >
          {copied ? <Check className="size-3" /> : <Copy className="size-3" />}
        </Button>
      </span>
    </TooltipProvider>
  );
}
