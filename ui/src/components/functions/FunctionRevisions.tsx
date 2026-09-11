import { useState, type ReactNode } from "react";
import { ConfirmDialog } from "@/components/sql-studio-v2/table-editor/ConfirmDialog";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { RevisionIdDisplay } from "@/components/functions/RevisionIdDisplay";
import { formatBytes } from "@/features/functions/format";
import { formatEpochMs, rtkErrorMessage } from "@/features/functions/display";
import { useRollbackModuleRevisionMutation } from "@/store/apiSlice";
import type { ModuleRevision } from "@/features/functions/types";

export function FunctionRevisions({
  moduleId,
  revisions,
}: {
  moduleId: string | null;
  revisions: ModuleRevision[];
}) {
  const [selected, setSelected] = useState<ModuleRevision | null>(null);
  const [pendingRollback, setPendingRollback] = useState<ModuleRevision | null>(null);
  const [rollbackError, setRollbackError] = useState<string | null>(null);
  const [rollbackModule, rollbackState] = useRollbackModuleRevisionMutation();

  const moduleRevisions = moduleId
    ? revisions
        .filter((revision) => revision.moduleId === moduleId)
        .slice()
        .sort((left, right) => right.createdAtMs - left.createdAtMs)
    : [];

  const confirmRollback = async () => {
    if (!moduleId || !pendingRollback) {
      return;
    }
    setRollbackError(null);
    try {
      await rollbackModule({ moduleId, revisionId: pendingRollback.revisionId }).unwrap();
      setPendingRollback(null);
      setSelected(null);
    } catch (error) {
      setRollbackError(rtkErrorMessage(error, "Failed to roll back module revision") ?? "Failed to roll back module revision");
    }
  };

  return (
    <div className="flex flex-col gap-4">
      <div>
        <h2 className="text-sm font-semibold">Revisions</h2>
        <p className="text-xs text-muted-foreground">Immutable module revisions from system.module_revisions.</p>
      </div>

      {!moduleId ? (
        <p className="py-8 text-center text-sm text-muted-foreground">
          This procedure is inline and has no module revisions.
        </p>
      ) : moduleRevisions.length === 0 ? (
        <p className="py-8 text-center text-sm text-muted-foreground">No revisions found for module {moduleId}.</p>
      ) : (
        <div className="rounded-md border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Revision</TableHead>
                <TableHead>Created</TableHead>
                <TableHead>Size</TableHead>
                <TableHead>Status</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {moduleRevisions.map((revision) => (
                <TableRow
                  key={revision.revisionId}
                  className="cursor-pointer"
                  onClick={() => {
                    setSelected(revision);
                    setRollbackError(null);
                  }}
                >
                  <TableCell>
                    <RevisionIdDisplay revisionId={revision.revisionId} />
                  </TableCell>
                  <TableCell className="text-muted-foreground">{formatEpochMs(revision.createdAtMs)}</TableCell>
                  <TableCell>{formatBytes(revision.artifactBytes)}</TableCell>
                  <TableCell>{revision.isCurrent ? "Current" : ""}</TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      )}

      <Sheet open={selected !== null} onOpenChange={(open) => !open && setSelected(null)}>
        <SheetContent className="w-full sm:max-w-md">
          <SheetHeader>
            <SheetTitle>Revision details</SheetTitle>
            <SheetDescription>
              {selected?.isCurrent ? "This revision is currently active." : "A previous immutable revision."}
            </SheetDescription>
          </SheetHeader>
          {selected ? (
            <div className="flex flex-col gap-4 px-6 pb-6">
              <Card>
                <CardHeader className="pb-2">
                  <CardTitle className="text-sm">Metadata</CardTitle>
                </CardHeader>
                <CardContent className="grid gap-3 text-sm">
                  <Detail label="Revision ID">
                    <RevisionIdDisplay revisionId={selected.revisionId} />
                  </Detail>
                  <Detail label="Artifact ID">
                    <span className="break-all font-mono text-xs">{selected.artifactId || "—"}</span>
                  </Detail>
                  <Detail label="Artifact size">{formatBytes(selected.artifactBytes)}</Detail>
                  <Detail label="Contract hash">
                    <span className="break-all font-mono text-xs">{selected.contractHash || "—"}</span>
                  </Detail>
                  <Detail label="Created">{formatEpochMs(selected.createdAtMs)}</Detail>
                  <Detail label="Exports">
                    {selected.exports.length > 0 ? selected.exports.join(", ") : "—"}
                  </Detail>
                  <Detail label="Status">{selected.isCurrent ? "Current" : "Previous"}</Detail>
                </CardContent>
              </Card>

              {rollbackError ? <p className="text-sm text-destructive">{rollbackError}</p> : null}

              {!selected.isCurrent && moduleId ? (
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => setPendingRollback(selected)}
                  disabled={rollbackState.isLoading}
                >
                  Rollback to this revision
                </Button>
              ) : null}
            </div>
          ) : null}
        </SheetContent>
      </Sheet>

      <ConfirmDialog
        open={pendingRollback !== null}
        title="Roll back to this revision?"
        description={
          <p>
            This will change the active revision for module &quot;{moduleId}&quot;. Existing revision history will not
            be changed.
          </p>
        }
        confirmLabel={rollbackState.isLoading ? "Rolling back…" : "Rollback"}
        onConfirm={() => {
          void confirmRollback();
        }}
        onClose={() => {
          if (!rollbackState.isLoading) {
            setPendingRollback(null);
          }
        }}
      />
    </div>
  );
}

function Detail({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="grid gap-1">
      <div className="text-xs text-muted-foreground">{label}</div>
      <div>{children}</div>
    </div>
  );
}
