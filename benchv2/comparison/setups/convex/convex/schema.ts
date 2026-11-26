import { defineSchema, defineTable } from "convex/server";
import { v } from "convex/values";

const message = {
  id: v.number(),
  owner: v.string(),
  room: v.string(),
  data: v.string(),
};

export default defineSchema({
  message: defineTable(message),
  realtime_message: defineTable(message).index("by_room", ["room"]),
});
