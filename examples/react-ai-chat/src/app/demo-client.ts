import {
  FileRef,
  LiveQueryController,
  type KalamDBClient,
  type LiveCallback,
  type LiveOptions,
  type LiveQueryDescriptor,
  type RowData,
  type Unsubscribe,
} from '@kalamdb/client';
import { buildApprovalMessage, buildAssistantReply, createToolPlan, splitIntoTokenChunks } from '../agent/logic';

type DemoRow = Record<string, unknown>;
type DemoState = Record<string, DemoRow[]>;
type Listener = {
  sql: string;
  callback: LiveCallback<unknown>;
  options: LiveOptions<unknown>;
};

const STORAGE_KEY = 'kalamdb-react-ai-chat-demo-v3';
const STREAM_DELAY_MS = 1_000;
const now = () => new Date();
const tableData = loadState();

export function createDemoClient(): KalamDBClient {
  const listeners = new Set<Listener>();

  const publish = () => {
    persistState();
    for (const listener of listeners) {
      listener.callback(mapLiveRows(selectRows(listener.sql), listener.options));
      listener.options.onCheckpoint?.({ subscriptionId: tableNameFromSql(listener.sql), lastSeqId: { toString: () => String(Date.now()) } as never });
    }
  };

  const client = {
    createLiveQueryController<TRow>(descriptor: LiveQueryDescriptor<TRow>) {
      return new LiveQueryController(this as unknown as KalamDBClient, descriptor);
    },
    async live(sql: string, callback: LiveCallback<unknown>, options: LiveOptions<unknown> = {}): Promise<Unsubscribe> {
      const listener = { sql, callback, options };
      listeners.add(listener);
      callback(mapLiveRows(selectRows(sql), options));
      return async () => {
        listeners.delete(listener);
      };
    },
    async insert(tableName: string, row: Record<string, unknown>) {
      const normalized = normalizeRow(row);
      const inserted = insertRow(tableName, normalized);
      touchConversationForRow(tableName, inserted);
      publish();
      afterInsert(tableName, inserted, publish);
      return { status: 'success', results: [] } as never;
    },
    async update(tableName: string, rowKey: string, patch: Record<string, unknown>) {
      const normalized = normalizeRow(patch);
      tableData[tableName] = (tableData[tableName] ?? []).map((row) => String(row.id) === rowKey
        ? { ...row, ...normalized, updated_at: now() }
        : row);
      publish();
      return { status: 'success', results: [] } as never;
    },
    async delete(tableName: string, rowKey: string) {
      tableData[tableName] = (tableData[tableName] ?? []).filter((row) => String(row.id) !== rowKey);
      publish();
    },
    async queryOne(sql: string, params?: unknown[]) {
      const tableName = tableNameFromSql(sql);
      const id = String(params?.[0] ?? whereValue(sql, 'id') ?? '');
      const row = (tableData[tableName] ?? []).find((entry) => String(entry.id) === id) ?? null;
      return row ? wrapQueryRow(row) : null;
    },
    async queryAll(sql: string, params?: unknown[]) {
      const tableName = tableNameFromSql(sql);
      const id = params?.[0] ? String(params[0]) : whereValue(sql, 'id');
      const rows = (tableData[tableName] ?? []).filter((entry) => !id || String(entry.id) === id);
      return rows.map(wrapQueryRow);
    },
    async queryWithFiles(_sql: string, files: Record<string, File | Blob>, params?: unknown[]) {
      const attachment = files.attachment;
      const fileName = attachment && 'name' in attachment ? attachment.name : 'attachment.bin';
      const row = insertRow('react_ai_chat.messages', {
        client_id: params?.[0],
        conversation_id: params?.[1],
        role: params?.[2],
        body: params?.[3],
        status: params?.[4],
        attachment: new FileRef({
          id: `demo-file-${Date.now()}`,
          sub: 'f0001',
          name: fileName,
          size: attachment?.size ?? 0,
          mime: attachment?.type || 'application/octet-stream',
          sha256: `demo-${attachment?.size ?? 0}-${Date.now()}`,
        }),
      });
      touchConversationForRow('react_ai_chat.messages', row);
      publish();
      afterInsert('react_ai_chat.messages', row, publish);
      return { status: 'success', results: [] } as never;
    },
  };

  return client as unknown as KalamDBClient;
}

function afterInsert(tableName: string, row: DemoRow, publish: () => void): void {
  if (tableName === 'react_ai_chat.messages' && row.role === 'user' && row.status === 'sent') {
    void simulateUserMessage(row, publish);
  }

  if (tableName === 'react_ai_chat.approval_actions') {
    void simulateApprovalAction(row, publish);
  }
}

async function simulateUserMessage(message: DemoRow, publish: () => void): Promise<void> {
  const body = String(message.body ?? '');
  const conversationId = String(message.conversation_id);
  const sourceMessageId = String(message.id);
  const plan = createToolPlan(body);

  await insertTyping(conversationId, `draft-${sourceMessageId}`, 'thinking', 'Thinking through the request. ', publish);

  if (plan.requiresApproval) {
    const approvalId = `approval-${sourceMessageId}`;
    insertRow('react_ai_chat.approvals', {
      id: approvalId,
      conversation_id: conversationId,
      message_id: sourceMessageId,
      title: plan.approvalTitle,
      body: plan.approvalBody,
      status: 'pending',
    });
    insertRow('react_ai_chat.messages', {
      conversation_id: conversationId,
      reply_to_message_id: sourceMessageId,
      role: 'assistant',
      body: buildApprovalMessage(body),
      status: 'awaiting_approval',
      approval_id: approvalId,
    });
    publish();
    return;
  }

  await streamThenInsertReply(conversationId, sourceMessageId, buildAssistantReply(body), publish);
}

async function simulateApprovalAction(actionRow: DemoRow, publish: () => void): Promise<void> {
  const approvalId = String(actionRow.approval_id);
  const action = String(actionRow.action);
  const conversationId = String(actionRow.conversation_id);
  const approval = findRow('react_ai_chat.approvals', approvalId);
  const sourceMessageId = String(approval?.message_id ?? approvalId);

  mutateRow('react_ai_chat.approvals', approvalId, { status: action, updated_at: now() });
  publish();

  if (action === 'declined') {
    insertRow('react_ai_chat.messages', {
      conversation_id: conversationId,
      reply_to_message_id: sourceMessageId,
      role: 'assistant',
      body: 'Approval was declined, so I stopped the action and left the workspace unchanged.',
      status: 'complete',
    });
    publish();
    return;
  }

  await streamThenInsertReply(conversationId, sourceMessageId, buildAssistantReply('approval granted'), publish);
}

async function streamThenInsertReply(conversationId: string, replyToMessageId: string, body: string, publish: () => void): Promise<void> {
  const draftMessageId = `draft-${replyToMessageId}-${Date.now()}`;
  for (const token of splitIntoTokenChunks(body)) {
    await insertTyping(conversationId, draftMessageId, 'typing', token, publish);
  }
  await insertTyping(conversationId, draftMessageId, 'saving', '', publish);
  insertRow('react_ai_chat.messages', {
    client_id: draftMessageId,
    conversation_id: conversationId,
    reply_to_message_id: replyToMessageId,
    role: 'assistant',
    body,
    status: 'complete',
  });
  publish();
}

async function insertTyping(conversationId: string, messageId: string, status: string, token: string, publish: () => void): Promise<void> {
  insertRow('react_ai_chat.typing_tokens', {
    conversation_id: conversationId,
    message_id: messageId,
    status,
    token,
  });
  publish();
  await delay(STREAM_DELAY_MS);
}

function selectRows(sql: string): RowData[] {
  const tableName = tableNameFromSql(sql);
  const conversationId = whereValue(sql, 'conversation_id');
  const rows = [...(tableData[tableName] ?? [])].filter((row) => !conversationId || row.conversation_id === conversationId);
  return rows as RowData[];
}

function mapLiveRows(rows: RowData[], options: LiveOptions<unknown>): unknown[] {
  return options.mapRow ? rows.map((row) => options.mapRow?.(row)) : rows;
}

function tableNameFromSql(sql: string): string {
  const matches = [...sql.matchAll(/from\s+([A-Za-z_][\w$]*(?:\.[A-Za-z_][\w$]*)?)/gi)];
  return matches[matches.length - 1]?.[1] ?? '';
}

function whereValue(sql: string, column: string): string | null {
  const match = sql.match(new RegExp(`${column}\\s*=\\s*'([^']+)'`, 'i'));
  return match?.[1] ?? null;
}

function insertRow(tableName: string, row: DemoRow): DemoRow {
  const id = String(row.id ?? demoId(tableName));
  const next = {
    ...row,
    id,
    created_at: row.created_at ?? now(),
    updated_at: row.updated_at ?? now(),
  };
  tableData[tableName] = [...(tableData[tableName] ?? []), next];
  return next;
}

function normalizeRow(row: Record<string, unknown>): Record<string, unknown> {
  return Object.fromEntries(Object.entries(row).map(([key, value]) => [toSnakeCase(key), value]));
}

function toSnakeCase(value: string): string {
  return value.replace(/[A-Z]/g, (letter) => `_${letter.toLowerCase()}`);
}

function demoId(tableName: string): string {
  const parts = tableName.split('.');
  const table = parts[parts.length - 1] ?? 'row';
  return `${table}-${Date.now()}-${Math.random().toString(16).slice(2, 7)}`;
}

function findRow(tableName: string, id: string): DemoRow | undefined {
  return (tableData[tableName] ?? []).find((row) => String(row.id) === id);
}

function mutateRow(tableName: string, id: string, patch: DemoRow): void {
  tableData[tableName] = (tableData[tableName] ?? []).map((row) => String(row.id) === id ? { ...row, ...patch } : row);
}

function touchConversationForRow(tableName: string, row: DemoRow): void {
  if (tableName !== 'react_ai_chat.messages' || typeof row.body !== 'string') {
    return;
  }

  const conversationId = row.conversation_id;
  const summary = row.body.slice(0, 92);
  tableData['react_ai_chat.conversations'] = (tableData['react_ai_chat.conversations'] ?? []).map((conversation) => conversation.id === conversationId
    ? { ...conversation, summary, updated_at: now() }
    : conversation);
}

function seedState(): DemoState {
  const seedTime = now();
  return {
    'react_ai_chat.conversations': [
      {
        id: 'project-alpha',
        title: 'Project Alpha',
        summary: 'Data analysis with approval-gated database migration',
        created_at: seedTime,
        updated_at: seedTime,
      },
      {
        id: 'market-research',
        title: 'Market Research',
        summary: 'Fresh conversation ready for realtime updates',
        created_at: seedTime,
        updated_at: seedTime,
      },
    ],
    'react_ai_chat.messages': [
      {
        id: '1001',
        client_id: null,
        conversation_id: 'project-alpha',
        reply_to_message_id: null,
        role: 'assistant',
        body: 'I analyzed the historical datasets and generated the quarterly summary chart. The European market outliers are isolated and ready for review.',
        status: 'complete',
        attachment: null,
        approval_id: 'approval-project-alpha',
        created_at: seedTime,
        updated_at: seedTime,
      },
    ],
    'react_ai_chat.approvals': [
      {
        id: 'approval-project-alpha',
        conversation_id: 'project-alpha',
        message_id: '1001',
        title: 'Action Required',
        body: 'Run database migration for Project Alpha?',
        status: 'pending',
        created_at: seedTime,
        updated_at: seedTime,
      },
    ],
    'react_ai_chat.typing_tokens': [],
    'react_ai_chat.approval_actions': [],
  };
}

function loadState(): DemoState {
  const seeded = seedState();
  const raw = window.localStorage.getItem(STORAGE_KEY);
  if (!raw) {
    return seeded;
  }

  try {
    const parsed = JSON.parse(raw) as Partial<DemoState>;
    return Object.fromEntries(Object.entries(seeded).map(([tableName, fallbackRows]) => [
      tableName,
      (parsed[tableName] ?? fallbackRows).map(rehydrateRow),
    ]));
  } catch {
    return seeded;
  }
}

function persistState(): void {
  window.localStorage.setItem(STORAGE_KEY, JSON.stringify(tableData));
}

function rehydrateRow(row: DemoRow): DemoRow {
  return Object.fromEntries(Object.entries(row).map(([key, value]) => {
    if (typeof value === 'string' && (key.endsWith('_at') || key === 'createdAt' || key === 'updatedAt')) {
      return [key, new Date(value)];
    }
    if (key === 'attachment' && value) {
      return [key, FileRef.from(value)];
    }
    return [key, value];
  }));
}

function wrapQueryRow(row: DemoRow): Record<string, unknown> {
  return Object.fromEntries(Object.entries(row).map(([key, value]) => [key, wrapCell(value)]));
}

function wrapCell(value: unknown): unknown {
  return {
    asString: () => value == null ? null : String(value),
    asDate: () => value instanceof Date ? value : new Date(String(value)),
    toJson: () => value,
  };
}

async function delay(ms: number): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, ms));
}