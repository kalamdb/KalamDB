//! Chat-message module for the comparison.
//!
//! Writes are the transactional reducer `insert_message`. Point reads are the
//! procedure `get_message`, which looks the row up by primary key and returns
//! it to the caller. That is a server round trip. It is not a client
//! subscription cache.

use spacetimedb::{ProcedureContext, ReducerContext, Table};

#[spacetimedb::table(accessor = message, public)]
pub struct Message {
    #[primary_key]
    id: i64,
    owner: String,
    room: String,
    data: String,
}

#[spacetimedb::reducer]
pub fn insert_message(ctx: &ReducerContext, id: i64, owner: String, room: String, data: String) {
    ctx.db.message().insert(Message {
        id,
        owner,
        room,
        data,
    });
}

#[spacetimedb::procedure]
pub fn get_message(ctx: &mut ProcedureContext, id: i64) -> Option<Message> {
    ctx.with_tx(|tx| tx.db.message().id().find(id))
}
