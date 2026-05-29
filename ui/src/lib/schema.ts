import { kTable } from '@kalamdb/orm';
import { boolean, doublePrecision, integer, jsonb, text, timestamp } from 'drizzle-orm/pg-core';
import { sql } from 'drizzle-orm';

export const dba_favorites = kTable('dba.favorites', {
  id: text('id').notNull(),
  payload: jsonb('payload'),
});

export const dba_notifications = kTable('dba.notifications', {
  id: text('id').notNull(),
  user_id: text('user_id').notNull(),
  title: text('title').notNull(),
  body: text('body'),
  is_read: boolean('is_read').default(sql``).notNull(),
  created_at: timestamp('created_at', { mode: 'string' }).notNull(),
  updated_at: timestamp('updated_at', { mode: 'string' }).notNull(),
});

export const system_audit_log = kTable('system.audit_log', {
  audit_id: text('audit_id').notNull(),
  timestamp: timestamp('timestamp', { mode: 'string' }).notNull(),
  actor_user_id: text('actor_user_id').notNull(),
  action: text('action').notNull(),
  target: text('target').notNull(),
  details: text('details'),
  ip_address: text('ip_address'),
  subject_user_id: text('subject_user_id'),
});

export const system_job_nodes = kTable('system.job_nodes', {
  job_id: text('job_id').notNull(),
  node_id: text('node_id').notNull(),
  status: text('status').notNull(),
  error_message: text('error_message'),
  created_at: timestamp('created_at', { mode: 'string' }).default(sql``).notNull(),
  started_at: timestamp('started_at', { mode: 'string' }),
  finished_at: timestamp('finished_at', { mode: 'string' }),
  updated_at: timestamp('updated_at', { mode: 'string' }).default(sql``).notNull(),
});

export const system_jobs = kTable('system.jobs', {
  job_id: text('job_id').notNull(),
  job_type: text('job_type').notNull(),
  status: text('status').notNull(),
  leader_status: text('leader_status'),
  parameters: jsonb('parameters'),
  message: text('message'),
  exception_trace: text('exception_trace'),
  idempotency_key: text('idempotency_key'),
  queue: text('queue'),
  priority: integer('priority'),
  retry_count: text('retry_count').notNull(),
  max_retries: text('max_retries').notNull(),
  memory_used: text('memory_used'),
  cpu_used: text('cpu_used'),
  created_at: timestamp('created_at', { mode: 'string' }).notNull(),
  updated_at: timestamp('updated_at', { mode: 'string' }).notNull(),
  started_at: timestamp('started_at', { mode: 'string' }),
  finished_at: timestamp('finished_at', { mode: 'string' }),
  node_id: text('node_id').notNull(),
  leader_node_id: text('leader_node_id'),
});

export const system_manifest = kTable('system.manifest', {
  cache_key: text('cache_key').notNull(),
  namespace_id: text('namespace_id').notNull(),
  table_name: text('table_name').notNull(),
  scope: text('scope').notNull(),
  etag: text('etag'),
  last_refreshed: timestamp('last_refreshed', { mode: 'string' }).notNull(),
  last_accessed: timestamp('last_accessed', { mode: 'string' }).notNull(),
  in_memory: boolean('in_memory').notNull(),
  sync_state: text('sync_state').notNull(),
  manifest_json: jsonb('manifest_json').notNull(),
});

export const system_namespaces = kTable('system.namespaces', {
  namespace_id: text('namespace_id').notNull(),
  name: text('name').notNull(),
  created_at: timestamp('created_at', { mode: 'string' }).notNull(),
  options: jsonb('options'),
  table_count: integer('table_count').notNull(),
});

export const system_schemas = kTable('system.schemas', {
  table_id: text('table_id').notNull(),
  table_name: text('table_name').notNull(),
  namespace_id: text('namespace_id').notNull(),
  table_type: text('table_type').notNull(),
  created_at: timestamp('created_at', { mode: 'string' }).notNull(),
  schema_version: integer('schema_version').notNull(),
  columns: jsonb('columns').notNull(),
  table_comment: text('table_comment'),
  updated_at: timestamp('updated_at', { mode: 'string' }).notNull(),
  options: jsonb('options'),
  access_level: text('access_level'),
  is_latest: boolean('is_latest').notNull(),
  storage_id: text('storage_id'),
  use_user_storage: boolean('use_user_storage'),
});

export const system_storages = kTable('system.storages', {
  storage_id: text('storage_id').notNull(),
  storage_name: text('storage_name').notNull(),
  description: text('description'),
  storage_type: text('storage_type').notNull(),
  base_directory: text('base_directory').notNull(),
  credentials: jsonb('credentials'),
  config_json: jsonb('config_json'),
  shared_tables_template: text('shared_tables_template').notNull(),
  user_tables_template: text('user_tables_template').notNull(),
  created_at: timestamp('created_at', { mode: 'string' }).notNull(),
  updated_at: timestamp('updated_at', { mode: 'string' }).notNull(),
});

export const system_topic_offsets = kTable('system.topic_offsets', {
  topic_id: text('topic_id').notNull(),
  group_id: text('group_id').notNull(),
  partition_id: integer('partition_id').notNull(),
  last_acked_offset: text('last_acked_offset').notNull(),
  updated_at: timestamp('updated_at', { mode: 'string' }).notNull(),
});

export const system_topics = kTable('system.topics', {
  topic_id: text('topic_id').notNull(),
  name: text('name').notNull(),
  alias: text('alias'),
  partitions: integer('partitions').notNull(),
  retention_seconds: text('retention_seconds'),
  retention_max_bytes: text('retention_max_bytes'),
  routes: jsonb('routes').notNull(),
  created_at: timestamp('created_at', { mode: 'string' }).notNull(),
  updated_at: timestamp('updated_at', { mode: 'string' }).notNull(),
});

export const system_users = kTable('system.users', {
  user_id: text('user_id').notNull(),
  password_hash: text('password_hash').notNull(),
  role: text('role').notNull(),
  email: text('email'),
  auth_type: text('auth_type').notNull(),
  auth_data: jsonb('auth_data'),
  storage_mode: text('storage_mode').notNull(),
  storage_id: text('storage_id'),
  failed_login_attempts: integer('failed_login_attempts').notNull(),
  locked_until: timestamp('locked_until', { mode: 'string' }),
  last_login_at: timestamp('last_login_at', { mode: 'string' }),
  created_at: timestamp('created_at', { mode: 'string' }).notNull(),
  updated_at: timestamp('updated_at', { mode: 'string' }).notNull(),
  last_seen: timestamp('last_seen', { mode: 'string' }),
  deleted_at: timestamp('deleted_at', { mode: 'string' }),
  invite_expires_at: timestamp('invite_expires_at', { mode: 'string' }),
  invited_by: text('invited_by'),
});

export const system_live = kTable('system.live', {
  live_id: text('live_id').notNull(),
  connection_id: text('connection_id').notNull(),
  subscription_id: text('subscription_id').notNull(),
  namespace_id: text('namespace_id').notNull(),
  table_name: text('table_name').notNull(),
  user_id: text('user_id').notNull(),
  query: text('query').notNull(),
  options: text('options'),
  status: text('status').notNull(),
  created_at: timestamp('created_at', { mode: 'string' }).notNull(),
  last_update: timestamp('last_update', { mode: 'string' }).notNull(),
  changes: text('changes').notNull(),
  node_id: text('node_id').notNull(),
  last_ping_at: timestamp('last_ping_at', { mode: 'string' }).notNull(),
});

export const system_server_logs = kTable('system.server_logs', {
  timestamp: text('timestamp').notNull(),
  level: text('level').notNull(),
  thread: text('thread'),
  target: text('target'),
  line: text('line'),
  message: text('message').notNull(),
});

export const system_slow_queries = kTable('system.slow_queries', {
  timestamp: text('timestamp').notNull(),
  timestamp_ms: integer('timestamp_ms').notNull(),
  duration_ms: doublePrecision('duration_ms').notNull(),
  user_id: text('user_id').notNull(),
  table_type: text('table_type').notNull(),
  table_name: text('table_name'),
  row_count: integer('row_count').notNull(),
  query: text('query').notNull(),
});

export const system_cluster = kTable('system.cluster', {
  cluster_id: text('cluster_id').notNull(),
  node_id: text('node_id').notNull(),
  role: text('role').notNull(),
  status: text('status').notNull(),
  rpc_addr: text('rpc_addr').notNull(),
  api_addr: text('api_addr').notNull(),
  is_self: boolean('is_self').notNull(),
  is_leader: boolean('is_leader').notNull(),
  groups_leading: integer('groups_leading').notNull(),
  total_groups: integer('total_groups').notNull(),
  current_term: text('current_term'),
  last_applied_log: text('last_applied_log'),
  leader_last_log_index: text('leader_last_log_index'),
  snapshot_index: text('snapshot_index'),
  catchup_progress_pct: text('catchup_progress_pct'),
  replication_lag: text('replication_lag'),
  hostname: text('hostname'),
  version: text('version'),
  memory_mb: text('memory_mb'),
  memory_usage_mb: text('memory_usage_mb'),
  cpu_usage_percent: text('cpu_usage_percent'),
  uptime_seconds: text('uptime_seconds'),
  uptime_human: text('uptime_human'),
  os: text('os'),
  arch: text('arch'),
});

export const system_settings = kTable('system.settings', {
  name: text('name').notNull(),
  value: text('value').notNull(),
  description: text('description').notNull(),
  category: text('category').notNull(),
});

export const system_stats = kTable('system.stats', {
  metric_name: text('metric_name').notNull(),
  metric_value: text('metric_value').notNull(),
});
