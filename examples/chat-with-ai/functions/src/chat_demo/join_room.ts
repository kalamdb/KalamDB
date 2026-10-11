import { eq, sql } from "drizzle-orm";
import {
  procedure,
  type JoinRoomRequest,
  roomMembers,
  rooms,
} from "../generated/contracts";

export const joinRoom = procedure.chatDemo.joinRoom(async (ctx, input: JoinRoomRequest) => {
  const roomId = input.room_id.trim();
  if (!roomId) {
    throw new Error("room_id is required");
  }

  const actor = ctx.actor.id;
  const existingRooms = await ctx.orm
    .select({ id: rooms.id })
    .from(rooms)
    .where(eq(rooms.id, roomId))
    .limit(1);
  if (existingRooms.length === 0) {
    await ctx.orm.insert(rooms).values({
      id: roomId,
      title: roomId,
      created_at: sql`DEFAULT`,
    });
  }

  const memberId = `${actor}:${roomId}`;
  const members = await ctx.orm
    .select({ id: roomMembers.id })
    .from(roomMembers)
    .where(eq(roomMembers.id, memberId))
    .limit(1);
  if (members.length === 0) {
    await ctx.orm.insert(roomMembers).values({
      id: memberId,
      user_id: actor,
      room_id: roomId,
    });
  }

  return roomId;
});
