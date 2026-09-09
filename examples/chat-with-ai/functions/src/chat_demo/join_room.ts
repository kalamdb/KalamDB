import { eq, sql } from "drizzle-orm";
import {
  defineProcedure,
  type ProcedureContext,
  type ChatDemoJoinRoomRequest,
  type ChatDemoJoinRoomResult,
  chatDemoRooms,
  chatDemoRoomMembers,
} from "../generated/contracts";

export default defineProcedure(
  async (ctx: ProcedureContext, input: ChatDemoJoinRoomRequest): Promise<ChatDemoJoinRoomResult> => {
    const roomId = input.room_id.trim();
    if (!roomId) {
      throw new Error("room_id is required");
    }

    const actor = ctx.actor.id;
    const rooms = await ctx.orm
      .select({ id: chatDemoRooms.id })
      .from(chatDemoRooms)
      .where(eq(chatDemoRooms.id, roomId))
      .limit(1);
    if (rooms.length === 0) {
      await ctx.orm.insert(chatDemoRooms).values({
        id: roomId,
        title: roomId,
        created_at: sql`DEFAULT`,
      });
    }

    const memberId = `${actor}:${roomId}`;
    const members = await ctx.orm
      .select({ id: chatDemoRoomMembers.id })
      .from(chatDemoRoomMembers)
      .where(eq(chatDemoRoomMembers.id, memberId))
      .limit(1);
    if (members.length === 0) {
      await ctx.orm.insert(chatDemoRoomMembers).values({
        id: memberId,
        user_id: actor,
        room_id: roomId,
      });
    }

    return roomId;
  },
);
