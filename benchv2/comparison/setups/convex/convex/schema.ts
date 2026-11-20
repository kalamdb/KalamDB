import { defineSchema, defineTable } from "convex/server";
import { v } from "convex/values";

export default defineSchema({
  message: defineTable({
    id: v.number(),
    owner: v.string(),
    room: v.string(),
    data: v.string(),
  }),
});
