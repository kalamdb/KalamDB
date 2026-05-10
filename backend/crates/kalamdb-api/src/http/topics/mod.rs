//! Topic pub/sub HTTP handlers
//!
//! This module provides REST API endpoints for topic consumption and acknowledgment.
//! These endpoints complement the SQL surface (CONSUME, ACK) with HTTP/JSON interfaces.
//!
//! ## Endpoints
//! - POST /v1/api/topics/consume - Consume messages from a topic
//! - POST /v1/api/topics/ack - Acknowledge offset for consumer group
//! - POST /v1/api/topics/latest-offsets - Resolve topic partition head offsets
//!
//! **Authorization**: Endpoints require `service`, `dba`, or `system` role (NOT `user`).

pub mod models;

mod ack;
mod consume;
mod latest_offsets;

pub(crate) use ack::ack_handler;
pub(crate) use consume::consume_handler;
pub(crate) use latest_offsets::latest_offsets_handler;
