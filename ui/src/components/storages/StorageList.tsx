import { useState } from 'react';
import {
  useCheckStorageHealthMutation,
  useGetStoragesQuery,
} from '@/store/apiSlice';
import type { Storage, StorageHealthResult } from '@/services/storageService';
import { StorageForm } from './StorageForm';
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table';
import { Button } from '@/components/ui/button';
import { Spinner } from '@/components/ui/spinner';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { RefreshCw, Database, HardDrive, Cloud, Plus, Pencil, Activity } from 'lucide-react';

interface StorageListProps {
  onSelectStorage?: (storage: Storage) => void;
}

export function StorageList({ onSelectStorage }: StorageListProps) {
  const {
    data: storages = [],
    isFetching: isLoading,
    error,
    refetch,
  } = useGetStoragesQuery();
  const [checkStorageHealthMutation] = useCheckStorageHealthMutation();
  const [showForm, setShowForm] = useState(false);
  const [editingStorage, setEditingStorage] = useState<Storage | undefined>(undefined);
  const [healthStorage, setHealthStorage] = useState<Storage | null>(null);
  const [healthResult, setHealthResult] = useState<StorageHealthResult | null>(null);
  const [healthError, setHealthError] = useState<string | null>(null);
  const [healthLoadingId, setHealthLoadingId] = useState<string | null>(null);

  const handleCreateNew = () => {
    setEditingStorage(undefined);
    setShowForm(true);
  };

  const handleEdit = (storage: Storage, e: React.MouseEvent) => {
    e.stopPropagation();
    setEditingStorage(storage);
    setShowForm(true);
  };

  const handleHealthCheck = async (storage: Storage, e: React.MouseEvent) => {
    e.stopPropagation();
    setHealthStorage(storage);
    setHealthResult(null);
    setHealthError(null);
    setHealthLoadingId(storage.storage_id);
    try {
      const result = await checkStorageHealthMutation({ storageId: storage.storage_id, extended: true }).unwrap();
      setHealthResult(result);
    } catch (err) {
      setHealthError(err instanceof Error ? err.message : 'Failed to check storage health');
    } finally {
      setHealthLoadingId(null);
    }
  };

  const closeHealthDialog = (open: boolean) => {
    if (!open) {
      setHealthStorage(null);
      setHealthResult(null);
      setHealthError(null);
    }
  };

  const handleFormSuccess = () => {
    setShowForm(false);
    setEditingStorage(undefined);
    void refetch();
  };

  const getStorageIcon = (type: string) => {
    switch (type.toLowerCase()) {
      case 's3':
      case 'cloud':
        return <Cloud className="h-4 w-4" />;
      case 'local':
      case 'filesystem':
        return <HardDrive className="h-4 w-4" />;
      default:
        return <Database className="h-4 w-4" />;
    }
  };

  const getStorageTypeBadge = (type: string) => {
    switch (type.toLowerCase()) {
      case 's3':
        return 'bg-orange-100 text-orange-800';
      case 'local':
      case 'filesystem':
        return 'bg-blue-100 text-blue-800';
      default:
        return 'bg-gray-100 text-gray-800';
    }
  };

  const getHealthStatusBadge = (status: string) => {
    switch (status.toLowerCase()) {
      case 'healthy':
        return 'bg-green-100 text-green-800';
      case 'degraded':
        return 'bg-yellow-100 text-yellow-800';
      default:
        return 'bg-red-100 text-red-800';
    }
  };

  const formatBytes = (bytes: number | null) => {
    if (bytes === null || Number.isNaN(bytes)) return 'N/A';
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
  };

  const formatTimestamp = (timestamp: number | null) => {
    if (!timestamp) return 'N/A';
    return new Date(timestamp).toLocaleString();
  };

  if (error) {
    const errorMessage =
      "error" in error && typeof error.error === "string"
        ? error.error
        : "Failed to fetch storages";
    return (
      <Card className="border-red-200">
        <CardContent className="py-6">
          <p className="text-red-700">{errorMessage}</p>
          <Button variant="outline" onClick={() => void refetch()} className="mt-2">
            Retry
          </Button>
        </CardContent>
      </Card>
    );
  }

  return (
    <div className="space-y-4">
      {/* Toolbar */}
      <div className="flex items-center justify-between">
        <Button onClick={handleCreateNew}>
          <Plus className="h-4 w-4 mr-2" />
          Create Storage
        </Button>
        <Button variant="outline" size="icon" onClick={() => void refetch()} disabled={isLoading} aria-label="Refresh storages">
          <RefreshCw className={`h-4 w-4 ${isLoading ? 'animate-spin' : ''}`} />
        </Button>
      </div>

      {/* Storage Form Dialog */}
      <StorageForm
        open={showForm}
        onOpenChange={setShowForm}
        storage={editingStorage}
        onSuccess={handleFormSuccess}
      />

      {/* Health Check Dialog */}
      <Dialog open={!!healthStorage} onOpenChange={closeHealthDialog}>
        <DialogContent className="max-w-xl">
          <DialogHeader>
            <DialogTitle>Storage Health</DialogTitle>
            <DialogDescription>
              {healthStorage ? `${healthStorage.storage_name} (${healthStorage.storage_id})` : ''}
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
            {healthLoadingId === healthStorage?.storage_id ? (
              <div className="flex items-center gap-2 text-sm text-muted-foreground">
                <Spinner className="size-4" />
                Running health checks...
              </div>
            ) : healthError ? (
              <div className="rounded-md border border-red-200 bg-red-50 px-3 py-2 text-sm text-red-700">
                {healthError}
              </div>
            ) : healthResult ? (
              <>
                <div className="flex items-center gap-3">
                  <span className={`px-2 py-1 rounded-full text-xs font-medium ${getHealthStatusBadge(healthResult.status)}`}>
                    {healthResult.status}
                  </span>
                  <span className="text-sm text-muted-foreground">
                    Latency: {healthResult.latency_ms} ms
                  </span>
                </div>
                <div className="grid grid-cols-2 gap-3 text-sm">
                  <div>Readable: {healthResult.readable ? 'Yes' : 'No'}</div>
                  <div>Writable: {healthResult.writable ? 'Yes' : 'No'}</div>
                  <div>Listable: {healthResult.listable ? 'Yes' : 'No'}</div>
                  <div>Deletable: {healthResult.deletable ? 'Yes' : 'No'}</div>
                  <div>Total: {formatBytes(healthResult.total_bytes)}</div>
                  <div>Used: {formatBytes(healthResult.used_bytes)}</div>
                  <div className="col-span-2">Tested at: {formatTimestamp(healthResult.tested_at)}</div>
                </div>
                {healthResult.error && (
                  <div className="rounded-md border border-yellow-200 bg-yellow-50 px-3 py-2 text-sm text-yellow-800">
                    {healthResult.error}
                  </div>
                )}
              </>
            ) : (
              <div className="text-sm text-muted-foreground">
                Health check results will appear here.
              </div>
            )}
          </div>
        </DialogContent>
      </Dialog>

      {/* Table */}
      {isLoading && storages.length === 0 ? (
        <div className="flex items-center justify-center py-8">
          <Spinner className="size-6 text-muted-foreground" />
        </div>
      ) : storages.length === 0 ? (
        <Card>
          <CardHeader>
            <CardTitle>No Storages Found</CardTitle>
            <CardDescription>
              No storage configurations are available.
            </CardDescription>
          </CardHeader>
        </Card>
      ) : (
        <div className="border rounded-lg">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Name</TableHead>
                <TableHead>Type</TableHead>
                <TableHead>Path</TableHead>
                <TableHead>Created</TableHead>
                <TableHead className="w-[80px]">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {storages.map((storage, index) => (
                <TableRow
                  key={storage.storage_id || `storage-${index}`}
                  className={onSelectStorage ? 'cursor-pointer' : ''}
                  onClick={() => onSelectStorage?.(storage)}
                >
                  <TableCell className="font-medium">
                    <div className="flex items-center gap-2">
                      {getStorageIcon(storage.storage_type)}
                      <div>
                        <div>{storage.storage_name}</div>
                        <div className="text-xs text-muted-foreground font-mono">
                          {storage.storage_id}
                        </div>
                      </div>
                    </div>
                  </TableCell>
                  <TableCell>
                    <span className={`px-2 py-1 rounded-full text-xs font-medium ${getStorageTypeBadge(storage.storage_type)}`}>
                      {storage.storage_type}
                    </span>
                  </TableCell>
                  <TableCell className="text-muted-foreground font-mono text-sm max-w-[300px] truncate">
                    {storage.base_directory}
                  </TableCell>
                  <TableCell className="text-muted-foreground">
                    {new Date(storage.created_at).toLocaleDateString()}
                  </TableCell>
                  <TableCell>
                    <div className="flex items-center gap-1">
                      <Button
                        variant="ghost"
                        size="icon"
                        onClick={(e) => handleHealthCheck(storage, e)}
                        title="Check storage health"
                        aria-label={`Check health for ${storage.storage_name}`}
                        disabled={healthLoadingId === storage.storage_id}
                      >
                        {healthLoadingId === storage.storage_id ? (
                          <Spinner className="size-4" />
                        ) : (
                          <Activity className="h-4 w-4" />
                        )}
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon"
                        onClick={(e) => handleEdit(storage, e)}
                        title="Edit storage"
                        aria-label={`Edit ${storage.storage_name}`}
                      >
                        <Pencil className="h-4 w-4" />
                      </Button>
                    </div>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      )}
    </div>
  );
}
