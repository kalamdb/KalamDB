import type { KalamDBClient } from '../client.js';
import type { RowData } from '../cell_value.js';
import type { LiveCheckpoint, LiveOptions, SubscriptionErrorEvent, Unsubscribe } from '../types.js';
import type { SeqId } from '../seq_id.js';
import type { LiveQueryDescriptor } from './descriptor.js';
import { projectLiveRows } from './projection.js';

export type LiveQueryControllerStatus = 'idle' | 'loading' | 'live' | 'reconnecting' | 'error' | 'disposed';

export interface LiveQueryControllerSnapshot<TRow> {
  rows: TRow[];
  loading: boolean;
  connected: boolean;
  status: LiveQueryControllerStatus;
  error?: Error;
  lastSeqId?: SeqId;
}

export type LiveQueryControllerListener<TRow> = (snapshot: LiveQueryControllerSnapshot<TRow>) => void;

export interface LiveQueryControllerOptions<TRow> extends Pick<LiveOptions<TRow>, 'batchSize' | 'lastRows' | 'from' | 'autoFetchBatches'> {
  onError?: (event: SubscriptionErrorEvent) => void;
  onCheckpoint?: (checkpoint: LiveCheckpoint) => void;
}

export class LiveQueryController<TRow = RowData> {
  readonly descriptor: LiveQueryDescriptor<TRow>;

  private readonly client: KalamDBClient;
  private readonly options: LiveQueryControllerOptions<TRow>;
  private readonly listeners = new Set<LiveQueryControllerListener<TRow>>();
  private unsubscribe?: Unsubscribe;
  private startToken = 0;
  private snapshot: LiveQueryControllerSnapshot<TRow> = {
    rows: [],
    loading: false,
    connected: false,
    status: 'idle',
  };

  constructor(
    client: KalamDBClient,
    descriptor: LiveQueryDescriptor<TRow>,
    options: LiveQueryControllerOptions<TRow> = {},
  ) {
    this.client = client;
    this.descriptor = descriptor;
    this.options = options;
  }

  getSnapshot(): LiveQueryControllerSnapshot<TRow> {
    return this.snapshot;
  }

  subscribe(listener: LiveQueryControllerListener<TRow>): Unsubscribe {
    this.listeners.add(listener);
    return async () => {
      this.listeners.delete(listener);
    };
  }

  async start(): Promise<void> {
    const token = this.startToken + 1;
    this.startToken = token;

    await this.stopActiveSubscription();
    this.setSnapshot({
      loading: true,
      connected: false,
      status: this.snapshot.rows.length > 0 ? 'reconnecting' : 'loading',
      error: undefined,
    });

    try {
      const unsubscribe = await this.client.live<TRow>(
        this.descriptor.subscriptionSql,
        (rows) => {
          if (this.startToken !== token) {
            return;
          }

          this.setSnapshot({
            rows: projectLiveRows(rows, this.descriptor.projection),
            loading: false,
            connected: true,
            status: 'live',
            error: undefined,
          });
        },
        this.liveOptions(),
      );

      if (this.startToken !== token) {
        await unsubscribe();
        return;
      }

      this.unsubscribe = unsubscribe;
      this.setSnapshot({ connected: true });
    } catch (error) {
      if (this.startToken !== token) {
        return;
      }
      this.setSnapshot({
        loading: false,
        connected: false,
        status: 'error',
        error: toError(error),
      });
    }
  }

  async refetch(): Promise<void> {
    await this.start();
  }

  async dispose(): Promise<void> {
    this.startToken += 1;
    await this.stopActiveSubscription();
    this.setSnapshot({
      loading: false,
      connected: false,
      status: 'disposed',
    });
    this.listeners.clear();
  }

  private liveOptions(): LiveOptions<TRow> {
    return {
      mapRow: this.descriptor.mapRow ?? ((row: RowData) => row as unknown as TRow),
      ...(this.descriptor.getKey ? { getKey: this.descriptor.getKey } : {}),
      ...(this.options.batchSize !== undefined ? { batchSize: this.options.batchSize } : {}),
      ...(this.options.lastRows !== undefined ? { lastRows: this.options.lastRows } : {}),
      ...(this.options.from !== undefined ? { from: this.options.from } : {}),
      ...(this.options.autoFetchBatches !== undefined ? { autoFetchBatches: this.options.autoFetchBatches } : {}),
      onError: (event) => {
        this.setSnapshot({
          loading: false,
          connected: false,
          status: 'error',
          error: new Error(event.message),
        });
        this.options.onError?.(event);
      },
      onCheckpoint: (checkpoint) => {
        this.setSnapshot({ lastSeqId: checkpoint.lastSeqId });
        this.options.onCheckpoint?.(checkpoint);
      },
    };
  }

  private async stopActiveSubscription(): Promise<void> {
    const unsubscribe = this.unsubscribe;
    this.unsubscribe = undefined;
    if (unsubscribe) {
      await unsubscribe();
    }
  }

  private setSnapshot(patch: Partial<LiveQueryControllerSnapshot<TRow>>): void {
    this.snapshot = {
      ...this.snapshot,
      ...patch,
    };

    for (const listener of this.listeners) {
      listener(this.snapshot);
    }
  }
}

function toError(error: unknown): Error {
  return error instanceof Error ? error : new Error(String(error));
}