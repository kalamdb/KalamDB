import { text, integer, timestamp, boolean, customType } from "drizzle-orm/pg-core";
import { kTable } from "@kalamdb/orm";

const upperJson = customType<{ data: Record<string, unknown>; driverData: string }>({
  dataType: () => "text",
  toDriver: (value) => JSON.stringify(value).toUpperCase(),
});

const params = new URLSearchParams(globalThis.location?.search ?? "");
const schemaName = `react_e2e_${params.get("schema") ?? "default"}`;

export const messages = kTable.user(`${schemaName}.messages`, {
  id: text("id").primaryKey(),
  roomId: text("room_id").notNull(),
  body: text("body").notNull(),
  authorName: text("author_name"),
  createdAt: timestamp("created_at"),
});

export const counters = kTable.user(`${schemaName}.counters`, {
  id: text("id").primaryKey(),
  value: integer("value").notNull(),
  isFavorite: boolean("is_favorite").notNull(),
});

export const encoded = kTable.user(`${schemaName}.encoded`, {
  id: text("id").primaryKey(),
  payload: upperJson("payload").notNull(),
});

export const schemaName_ = schemaName;
