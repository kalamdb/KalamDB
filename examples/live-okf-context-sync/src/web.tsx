import { createRoot } from 'react-dom/client';
import { useEffect, useMemo, useState } from 'react';
import { desc, eq } from 'drizzle-orm';
import { liveTable } from '@kalamdb/orm';
import { Auth, createClient, type SubscriptionErrorEvent } from '@kalamdb/client';
import { createDb, NAMESPACE } from './client.js';
import { context_files, type ContextFiles } from './schema.generated.js';
import './web.css';

type UserName = 'alice' | 'bob';

type FileRow = {
  path: string;
  sha256: string;
  mimeType: string;
  sizeBytes: number;
  isConflict: boolean;
  canonicalPath: string | null;
  deleted: boolean;
  updatedAt: string;
};

const USERS: Record<UserName, { password: string; folder: string }> = {
  alice: { password: 'alice123', folder: 'context/alice' },
  bob: { password: 'bob123', folder: 'context/bob' },
};

function toFileRow(row: ContextFiles): FileRow {
  return {
    path: row.path,
    sha256: row.sha256,
    mimeType: row.mime_type,
    sizeBytes: Number(row.size_bytes ?? 0),
    isConflict: row.is_conflict ?? false,
    canonicalPath: row.canonical_path,
    deleted: row.deleted ?? false,
    updatedAt: row.updated_at ? row.updated_at.toISOString() : '',
  };
}

function statusLabel(row: FileRow): string {
  if (row.deleted) {
    return 'deleted';
  }
  if (row.isConflict) {
    return 'conflict';
  }
  return 'synced';
}

function formatUpdated(value: string): string {
  if (!value) {
    return '—';
  }

  const parsed = Date.parse(value);
  if (Number.isNaN(parsed)) {
    return value;
  }

  const seconds = Math.max(0, Math.round((Date.now() - parsed) / 1000));
  if (seconds < 5) {
    return 'now';
  }
  if (seconds < 60) {
    return `${seconds}s ago`;
  }

  const minutes = Math.round(seconds / 60);
  return `${minutes}m ago`;
}

function App() {
  const [user, setUser] = useState<UserName>('alice');
  const [rows, setRows] = useState<FileRow[]>([]);
  const [status, setStatus] = useState<'loading' | 'live' | 'error'>('loading');
  const [error, setError] = useState<string | null>(null);
  const [preview, setPreview] = useState<string | null>(null);
  const [previewPath, setPreviewPath] = useState<string | null>(null);

  const url = import.meta.env.VITE_KALAM_URL ?? 'http://127.0.0.1:2900';

  useEffect(() => {
    let active = true;
    let unsubscribe: (() => Promise<void>) | undefined;
    const credentials = USERS[user];

    const client = createClient({
      url,
      namespace: NAMESPACE,
      authProvider: async () => Auth.basic(user, credentials.password),
      disableCompression: true,
    });

    const start = async (): Promise<void> => {
      setStatus('loading');
      setError(null);
      setPreview(null);
      setPreviewPath(null);

      try {
        unsubscribe = await liveTable(
          client,
          context_files,
          (liveRows) => {
            if (!active) {
              return;
            }

            setRows(liveRows.map(toFileRow));
            setStatus('live');
          },
          {
            orderBy: desc(context_files.updated_at),
            onError: (event: SubscriptionErrorEvent) => {
              if (!active) {
                return;
              }

              setStatus('error');
              setError(`Subscription dropped (${event.code}): ${event.message}`);
            },
          },
        );
      } catch (startError) {
        if (!active) {
          return;
        }

        setStatus('error');
        setError(startError instanceof Error ? startError.message : String(startError));
      }
    };

    void start();

    return () => {
      active = false;
      void unsubscribe?.();
      void client.disconnect();
    };
  }, [url, user]);

  const summary = useMemo(() => {
    const synced = rows.filter((row) => !row.deleted && !row.isConflict).length;
    const conflicts = rows.filter((row) => row.isConflict).length;
    const deleted = rows.filter((row) => row.deleted).length;
    return { synced, conflicts, deleted };
  }, [rows]);

  async function previewFile(row: FileRow): Promise<void> {
    const credentials = USERS[user];
    const client = createClient({
      url,
      namespace: NAMESPACE,
      authProvider: async () => Auth.basic(user, credentials.password),
      disableCompression: true,
    });

    try {
      await client.initialize();
      const db = createDb(client);
      const login = await client.login();
      const queryRows = await db
        .select({ file_ref: context_files.file_ref })
        .from(context_files)
        .where(eq(context_files.path, row.path));
      const fileRef = queryRows[0]?.file_ref;
      if (!fileRef) {
        throw new Error('missing file_ref');
      }

      const response = await fetch(fileRef.getDownloadUrl(url, NAMESPACE, 'context_files'), {
        headers: {
          Authorization: `Bearer ${login.access_token}`,
        },
      });

      if (!response.ok) {
        throw new Error(`download failed (${response.status})`);
      }

      const text = await response.text();
      setPreview(text);
      setPreviewPath(row.path);
    } catch (previewError) {
      setError(previewError instanceof Error ? previewError.message : String(previewError));
    } finally {
      await client.disconnect();
    }
  }

  return (
    <main className="page">
      <header className="hero">
        <div>
          <p className="eyebrow">KalamDB showcase</p>
          <h1>Live OKF Context Sync</h1>
          <p className="lede">
            One SQL table, local Markdown files, and live metadata subscriptions. This example is a
            showcase app — KalamDB does not implement folder sync internally.
          </p>
        </div>
        <div className="panel">
          <label htmlFor="user-select">Current user</label>
          <select
            id="user-select"
            value={user}
            onChange={(event) => setUser(event.target.value as UserName)}
          >
            <option value="alice">alice</option>
            <option value="bob">bob</option>
          </select>
          <p className="meta">Watched folder: {USERS[user].folder}</p>
          <p className="meta">Connection: {status === 'live' ? 'live' : status}</p>
        </div>
      </header>

      <section className="stats">
        <div><strong>{summary.synced}</strong><span>synced</span></div>
        <div><strong>{summary.conflicts}</strong><span>conflicts</span></div>
        <div><strong>{summary.deleted}</strong><span>deleted</span></div>
      </section>

      {error ? <p className="error">{error}</p> : null}

      <section className="table-wrap">
        <table>
          <thead>
            <tr>
              <th>Path</th>
              <th>Status</th>
              <th>Updated</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => (
              <tr key={row.path} className={row.deleted ? 'deleted' : row.isConflict ? 'conflict' : ''}>
                <td>
                  <code>{row.path}</code>
                  {row.canonicalPath ? <span className="subtle">canonical: {row.canonicalPath}</span> : null}
                </td>
                <td>{statusLabel(row)}</td>
                <td>{formatUpdated(row.updatedAt)}</td>
                <td>
                  {!row.deleted ? (
                    <button type="button" onClick={() => void previewFile(row)}>
                      Preview
                    </button>
                  ) : null}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>

      {preview ? (
        <section className="preview">
          <div className="preview-header">
            <h2>{previewPath}</h2>
            <button type="button" onClick={() => { setPreview(null); setPreviewPath(null); }}>
              Close
            </button>
          </div>
          <pre>{preview}</pre>
        </section>
      ) : null}
    </main>
  );
}

createRoot(document.getElementById('root')!).render(<App />);
