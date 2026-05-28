import { useEffect, useMemo, useState } from "react";
import { Download, FileArchive, ListFilter } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { useToast } from "@/components/ui/toaster-provider";
import type { StudioTable } from "@/components/sql-studio-v2/shared/types";
import {
  downloadTableExportArchive,
  getTableExportStatus,
    isTerminalTableTransferStatus,
  startTableExport,
  type TableTransferJobResponse,
  type TableTransferInput,
} from "@/services/tableTransferService";
import { sectionTitleClassName, fieldLabelClassName } from "@/components/layout/typography";

interface TableDataTransferActionsProps {
  table: StudioTable;
  onOpenQueryInNewTab?: (query: string, title: string) => void;
}

function normalizeTableType(tableType: string): "user" | "shared" | null {
  const normalized = tableType.toLowerCase();
  if (normalized === "user") return "user";
  if (normalized === "shared") return "shared";
  return null;
}

function buildTransferInput(table: StudioTable, userId: string): TableTransferInput | null {
  const tableType = normalizeTableType(table.tableType);
  if (!tableType) return null;
  return {
    namespace_id: table.namespace,
    table_name: table.name,
    table_type: tableType,
    user_id: tableType === "user" ? userId.trim() : undefined,
  };
}

function transferStatusLabel(job: TableTransferJobResponse | null): string {
  if (!job) return "Idle";
  if (job.message?.trim()) return `${job.status}: ${job.message}`;
  return job.status;
}

export function TableDataTransferActions({
  table,
  onOpenQueryInNewTab,
}: TableDataTransferActionsProps) {
  const tableType = useMemo(() => normalizeTableType(table.tableType), [table.tableType]);
  const { notify } = useToast();
  const [exportOpen, setExportOpen] = useState(false);
  const [exportUserId, setExportUserId] = useState("");
  const [exportJob, setExportJob] = useState<TableTransferJobResponse | null>(null);
  const [exportBusy, setExportBusy] = useState(false);
  const [downloadBusy, setDownloadBusy] = useState(false);

  useEffect(() => {
    if (!exportJob || isTerminalTableTransferStatus(exportJob.status)) return;
    let cancelled = false;
    const timer = window.setInterval(() => {
      void getTableExportStatus(exportJob.job_id)
        .then((nextJob) => {
          if (cancelled) return;
          setExportJob(nextJob);
          if (nextJob.status === "completed") {
            notify({ title: "Table export is ready", variant: "success" });
          } else if (isTerminalTableTransferStatus(nextJob.status)) {
            notify({ title: "Table export finished", description: nextJob.message, variant: "default" });
          }
        })
        .catch((error) => {
          if (!cancelled) {
            notify({
              title: "Failed to refresh export status",
              description: error instanceof Error ? error.message : String(error),
              variant: "destructive",
            });
          }
        });
    }, 1500);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [exportJob?.job_id, exportJob?.status, notify]);

  if (!tableType) {
    return null;
  }

  const fqn = `${table.namespace}.${table.name}`;
  const needsUserId = tableType === "user";
  const exportCanSubmit = !needsUserId || exportUserId.trim().length > 0;

  const openExportsQuery = () => {
    onOpenQueryInNewTab?.(
      "SELECT job_id, job_type, status, message, parameters, created_at, finished_at\nFROM system.jobs\nWHERE job_type IN ('table_export', 'table_import')\nORDER BY created_at DESC\nLIMIT 20;",
      "Table transfers",
    );
  };

  const handleStartExport = async () => {
    const input = buildTransferInput(table, exportUserId);
    if (!input) return;
    setExportBusy(true);
    try {
      const job = await startTableExport(input);
      setExportJob(job);
      notify({ title: "Table export started", variant: "success" });
    } catch (error) {
      notify({
        title: "Failed to start export",
        description: error instanceof Error ? error.message : String(error),
        variant: "destructive",
      });
    } finally {
      setExportBusy(false);
    }
  };

  const handleDownload = async () => {
    if (!exportJob?.download_url) return;
    setDownloadBusy(true);
    try {
      await downloadTableExportArchive(
        exportJob.download_url,
        `${exportJob.export_id ?? "table-export"}.zip`,
      );
    } catch (error) {
      notify({
        title: "Failed to download export",
        description: error instanceof Error ? error.message : String(error),
        variant: "destructive",
      });
    } finally {
      setDownloadBusy(false);
    }
  };

  return (
    <section className="space-y-2">
      <div className="flex items-center justify-between gap-3">
        <h3 className={sectionTitleClassName}>Data</h3>
        <div className="flex items-center gap-2">
          <Button type="button" variant="outline" size="sm" className="gap-1.5" onClick={openExportsQuery}>
            <ListFilter data-icon="inline-start" />
            View Jobs
          </Button>
          <Button type="button" size="sm" className="gap-1.5" onClick={() => setExportOpen(true)}>
            <FileArchive data-icon="inline-start" />
            Export
          </Button>
        </div>
      </div>

      <div className="rounded-md border border-border bg-muted/20 px-4 py-3 text-xs text-muted-foreground">
        {exportJob ? (
          <div className="truncate" title={transferStatusLabel(exportJob)}>
            Export: {transferStatusLabel(exportJob)}
          </div>
        ) : (
          <span>{fqn}</span>
        )}
      </div>

      <Dialog open={exportOpen} onOpenChange={setExportOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Export {fqn}</DialogTitle>
            <DialogDescription>
              {needsUserId ? "Enter the user scope for this user table." : "Shared table export."}
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-3">
            {needsUserId && (
              <label className="flex flex-col gap-1.5">
                <span className={fieldLabelClassName}>User ID</span>
                <Input value={exportUserId} onChange={(event) => setExportUserId(event.target.value)} className="h-9" />
              </label>
            )}
            {exportJob?.status === "completed" && exportJob.download_url && (
              <Button type="button" className="w-full gap-1.5" onClick={handleDownload} disabled={downloadBusy}>
                <Download data-icon="inline-start" />
                Download ZIP
              </Button>
            )}
            {exportJob && (
              <p className="text-xs text-muted-foreground">{transferStatusLabel(exportJob)}</p>
            )}
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={openExportsQuery}>View Jobs</Button>
            <Button type="button" onClick={handleStartExport} disabled={!exportCanSubmit || exportBusy}>
              {exportBusy ? "Starting..." : "Start Export"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

    </section>
  );
}
