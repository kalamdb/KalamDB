import { LiveQueries, LiveQuery, KalamProvider } from '@kalamdb/react';
import type { SingleLiveQueryContext } from '@kalamdb/react';
import type { RowData } from '@kalamdb/client';
import { kTable } from '@kalamdb/orm';
import { bigint, integer, text, timestamp } from 'drizzle-orm/pg-core';
import { desc } from 'drizzle-orm';
import { Activity, DatabaseZap, Radio } from 'lucide-react';
import { getClient } from '@/lib/kalam-client';
import { Card } from '@/components/ui/card';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';

const liveQueryRows = kTable.system('system.live_queries', {
  liveId: text('live_id').primaryKey(),
  subscriptionId: text('subscription_id').notNull(),
  tableName: text('table_name').notNull(),
  status: text('status').notNull(),
  createdAt: timestamp('created_at', { mode: 'date' }),
});

const jobRows = kTable.system('system.jobs', {
  jobId: text('job_id').primaryKey(),
  jobType: text('job_type').notNull(),
  status: text('status').notNull(),
  attempts: integer('attempts'),
});

const storageRows = kTable.system('system.storages', {
  storageId: bigint('storage_id', { mode: 'number' }).primaryKey(),
  storageName: text('storage_name').notNull(),
  storageType: text('storage_type').notNull(),
});

export function ReactLiveQueryDemo() {
  const client = getClient();

  if (!client) {
    return (
      <Card className="p-4">
        <div className="flex items-center gap-3">
          <Radio className="h-5 w-5 text-muted-foreground" />
          <div>
            <h2 className="text-base font-semibold">React live data pilot</h2>
            <p className="text-sm text-muted-foreground">Log in to initialize the SDK client and open the React live-query demo.</p>
          </div>
        </div>
      </Card>
    );
  }

  return (
    <KalamProvider client={client}>
      <div className="grid gap-4 xl:grid-cols-2">
        <Card className="p-4">
          <div className="mb-3 flex items-center justify-between gap-3">
            <div className="flex items-center gap-2">
              <Activity className="h-5 w-5 text-green-600" />
              <h2 className="text-base font-semibold">Component LiveQuery</h2>
            </div>
            <Badge variant="outline">SQL mode</Badge>
          </div>
          <LiveQuery query="SELECT * FROM system.live_queries ORDER BY created_at DESC LIMIT 5" getKey="live_id">
            {({ rows, state, refetch }: SingleLiveQueryContext<RowData>) => (
              <div className="space-y-3">
                <div className="flex items-center justify-between text-sm text-muted-foreground">
                  <span>{state.loading ? 'Opening stream' : `${rows.length} recent subscriptions`}</span>
                  <Button type="button" variant="outline" size="sm" onClick={() => void refetch()}>Refresh</Button>
                </div>
                <div className="grid gap-2">
                  {rows.map((row) => (
                    <div key={row.live_id?.asString?.() ?? String(row.live_id)} className="rounded border p-2 text-sm">
                      <div className="font-medium">{row.table_name?.asString?.() ?? String(row.table_name ?? 'unknown table')}</div>
                      <div className="text-muted-foreground">{row.status?.asString?.() ?? String(row.status ?? 'unknown')}</div>
                    </div>
                  ))}
                  {!state.loading && rows.length === 0 ? <p className="text-sm text-muted-foreground">No active rows yet.</p> : null}
                </div>
              </div>
            )}
          </LiveQuery>
        </Card>

        <Card className="p-4">
          <div className="mb-3 flex items-center justify-between gap-3">
            <div className="flex items-center gap-2">
              <DatabaseZap className="h-5 w-5 text-blue-600" />
              <h2 className="text-base font-semibold">Hook LiveQueries</h2>
            </div>
            <Badge variant="outline">Typed mode</Badge>
          </div>
          <LiveQueries
            queries={{
              subscriptions: {
                table: liveQueryRows,
                orderBy: (table) => desc(table.createdAt),
                limit: 5,
              },
              jobs: { table: jobRows, limit: 5 },
              storages: { table: storageRows, limit: 5 },
            }}
          >
            {({ subscriptions, jobs, storages, state }) => (
              <div className="grid gap-3 sm:grid-cols-3">
                <Metric label="Subscriptions" value={subscriptions.rows.length} loading={state.loading} />
                <Metric label="Jobs" value={jobs.rows.length} loading={state.loading} />
                <Metric label="Storages" value={storages.rows.length} loading={state.loading} />
              </div>
            )}
          </LiveQueries>
        </Card>
      </div>
    </KalamProvider>
  );
}

function Metric({ label, value, loading }: { label: string; value: number; loading: boolean }) {
  return (
    <div className="rounded border p-3">
      <div className="text-sm text-muted-foreground">{label}</div>
      <div className="text-2xl font-semibold">{loading ? '...' : value}</div>
    </div>
  );
}