import { v } from "convex/values";
import { mutation, query } from "./_generated/server";

export const insertMessage = mutation({
  args: {
    id: v.number(),
    owner: v.string(),
    room: v.string(),
    data: v.string(),
  },
  returns: v.id("message"),
  handler: async (ctx, args) => {
    return await ctx.db.insert("message", args);
  },
});

export const getMessage = query({
  args: { id: v.id("message") },
  returns: v.union(
    v.object({
      _id: v.id("message"),
      _creationTime: v.number(),
      id: v.number(),
      owner: v.string(),
      room: v.string(),
      data: v.string(),
    }),
    v.null(),
  ),
  handler: async (ctx, args) => {
    return await ctx.db.get(args.id);
  },
});

export const insertRealtimeMessage = mutation({
  args: {
    id: v.number(),
    owner: v.string(),
    room: v.string(),
    data: v.string(),
  },
  returns: v.id("realtime_message"),
  handler: async (ctx, args) => {
    return await ctx.db.insert("realtime_message", args);
  },
});

export const roomMessages = query({
  args: { room: v.string() },
  handler: async (ctx, { room }) => {
    return await ctx.db
      .query("realtime_message")
      .withIndex("by_room", (q) => q.eq("room", room))
      .collect();
  },
});
