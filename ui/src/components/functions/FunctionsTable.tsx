import { MoreHorizontal } from "lucide-react";
import { useNavigate } from "react-router-dom";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { FunctionStatusBadge } from "@/components/functions/FunctionStatusBadge";
import { RevisionIdDisplay } from "@/components/functions/RevisionIdDisplay";
import { formatCount, formatEpochMs } from "@/features/functions/display";
import { functionDetailPath } from "@/features/functions/paths";
import type { ProcedureListItem } from "@/features/functions/types";

export function FunctionsTable({
  items,
  isLoading,
}: {
  items: ProcedureListItem[];
  isLoading: boolean;
}) {
  const navigate = useNavigate();

  return (
    <div className="rounded-md border">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Function</TableHead>
            <TableHead>Status</TableHead>
            <TableHead>Current revision</TableHead>
            <TableHead>Last updated</TableHead>
            <TableHead className="text-right">Calls 24h</TableHead>
            <TableHead className="w-12 text-right">Actions</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {isLoading && items.length === 0 ? (
            <TableRow>
              <TableCell colSpan={6} className="py-10 text-center text-sm text-muted-foreground">
                Loading functions…
              </TableCell>
            </TableRow>
          ) : items.length === 0 ? (
            <TableRow>
              <TableCell colSpan={6} className="py-10 text-center text-sm text-muted-foreground">
                No functions found.
              </TableCell>
            </TableRow>
          ) : (
            items.map((item) => (
              <TableRow
                key={item.id}
                className="cursor-pointer"
                onClick={() => navigate(functionDetailPath(item.id))}
              >
                <TableCell>
                  <div className="flex min-w-0 flex-col">
                    <span className="font-medium">{item.id}</span>
                    {item.comment ? (
                      <span className="truncate text-xs text-muted-foreground">{item.comment}</span>
                    ) : null}
                  </div>
                </TableCell>
                <TableCell>
                  <FunctionStatusBadge status={item.status} />
                </TableCell>
                <TableCell>
                  <RevisionIdDisplay revisionId={item.revisionId} />
                </TableCell>
                <TableCell className="text-muted-foreground">{formatEpochMs(item.lastUpdatedMs)}</TableCell>
                <TableCell className="text-right tabular-nums">{formatCount(item.calls24h)}</TableCell>
                <TableCell className="text-right">
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon-xs"
                        aria-label={`Actions for ${item.id}`}
                        onClick={(event) => event.stopPropagation()}
                      >
                        <MoreHorizontal />
                      </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end" onClick={(event) => event.stopPropagation()}>
                      <DropdownMenuItem onSelect={() => navigate(functionDetailPath(item.id))}>
                        Open
                      </DropdownMenuItem>
                      <DropdownMenuItem
                        onSelect={() => {
                          void navigator.clipboard.writeText(item.id);
                        }}
                      >
                        Copy ID
                      </DropdownMenuItem>
                    </DropdownMenuContent>
                  </DropdownMenu>
                </TableCell>
              </TableRow>
            ))
          )}
        </TableBody>
      </Table>
    </div>
  );
}
