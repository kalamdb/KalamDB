import { api } from "@/lib/api";
import { getApiBaseUrl, getBackendOrigin } from "@/lib/backend-url";
import { getCurrentToken } from "@/lib/kalam-client";

export type TableTransferStatus =
  | "new"
  | "queued"
  | "running"
  | "retrying"
  | "completed"
  | "failed"
  | "cancelled"
  | "skipped";

export interface TableTransferInput {
  namespace_id: string;
  table_name: string;
  table_type: "user" | "shared";
  user_id?: string;
}

export interface TableTransferJobResponse {
  job_id: string;
  export_id?: string;
  import_id?: string;
  status: TableTransferStatus;
  message?: string;
  download_url?: string;
}

function authHeaders(): Headers {
  const headers = new Headers();
  const token = getCurrentToken();
  if (token) {
    headers.set("Authorization", `Bearer ${token}`);
  }
  return headers;
}

async function parseError(response: Response): Promise<string> {
  const payload = await response.json().catch(() => null) as { message?: unknown; error?: unknown } | null;
  if (typeof payload?.message === "string") {
    return payload.message;
  }
  if (typeof payload?.error === "string") {
    return payload.error;
  }
  return `Request failed with status ${response.status}`;
}

export function startTableExport(input: TableTransferInput): Promise<TableTransferJobResponse> {
  return api.post<TableTransferJobResponse>("/table-exports", input);
}

export function getTableExportStatus(jobId: string): Promise<TableTransferJobResponse> {
  return api.get<TableTransferJobResponse>(`/table-exports/${encodeURIComponent(jobId)}`);
}

export async function startTableImport(
  input: TableTransferInput,
  file: File,
): Promise<TableTransferJobResponse> {
  const form = new FormData();
  form.set("namespace_id", input.namespace_id);
  form.set("table_name", input.table_name);
  form.set("table_type", input.table_type);
  if (input.user_id?.trim()) {
    form.set("user_id", input.user_id.trim());
  }
  form.set("file", file, file.name);

  const response = await fetch(`${getApiBaseUrl()}/table-imports`, {
    method: "POST",
    credentials: "include",
    headers: authHeaders(),
    body: form,
  });

  if (!response.ok) {
    throw new Error(await parseError(response));
  }

  return response.json() as Promise<TableTransferJobResponse>;
}

export function getTableImportStatus(jobId: string): Promise<TableTransferJobResponse> {
  return api.get<TableTransferJobResponse>(`/table-imports/${encodeURIComponent(jobId)}`);
}

export async function downloadTableExportArchive(
  downloadUrl: string,
  filename: string,
): Promise<void> {
  const url = downloadUrl.startsWith("http")
    ? downloadUrl
    : `${getBackendOrigin()}${downloadUrl.startsWith("/") ? downloadUrl : `/${downloadUrl}`}`;
  const response = await fetch(url, {
    method: "GET",
    credentials: "include",
    headers: authHeaders(),
  });

  if (!response.ok) {
    throw new Error(await parseError(response));
  }

  const blob = await response.blob();
  const objectUrl = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = objectUrl;
  anchor.download = filename;
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  window.setTimeout(() => URL.revokeObjectURL(objectUrl), 1000);
}

export function isTerminalTableTransferStatus(status: TableTransferStatus): boolean {
  return ["completed", "failed", "cancelled", "skipped"].includes(status);
}
