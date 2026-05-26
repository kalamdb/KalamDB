import {
  dba_favorites,
  dba_notifications,
  system_audit_log,
  system_cluster,
  system_jobs,
  system_live,
  system_namespaces,
  system_schemas,
  system_server_logs,
  system_settings,
  system_slow_queries,
  system_stats,
  system_storages,
  system_topic_offsets,
  system_topics,
  system_users,
} from "@/lib/schema";

export type DbaFavoriteRow = typeof dba_favorites.$inferSelect;
export type DbaNotificationRow = typeof dba_notifications.$inferSelect;
export type SystemAuditLogRow = typeof system_audit_log.$inferSelect;
export type SystemClusterNodeRow = typeof system_cluster.$inferSelect;
export type SystemJobRow = typeof system_jobs.$inferSelect;
export type SystemLiveQueryRow = typeof system_live.$inferSelect;
export type SystemNamespaceRow = typeof system_namespaces.$inferSelect;
export type SystemSchemaRow = typeof system_schemas.$inferSelect;
export type SystemServerLogRow = typeof system_server_logs.$inferSelect;
export type SystemSettingRow = typeof system_settings.$inferSelect;
export type SystemSlowQueryRow = typeof system_slow_queries.$inferSelect;
export type SystemStatRow = typeof system_stats.$inferSelect;
export type SystemStorageRow = typeof system_storages.$inferSelect;
export type SystemTopicOffsetRow = typeof system_topic_offsets.$inferSelect;
export type SystemTopicRow = typeof system_topics.$inferSelect;
export type SystemUserRow = typeof system_users.$inferSelect;

export type SystemUserListRow = Pick<SystemUserRow,
  | "user_id"
  | "role"
  | "email"
  | "auth_type"
  | "auth_data"
  | "storage_mode"
  | "storage_id"
  | "created_at"
  | "updated_at"
  | "last_seen"
  | "deleted_at"
  | "failed_login_attempts"
  | "locked_until"
  | "last_login_at"
>;
