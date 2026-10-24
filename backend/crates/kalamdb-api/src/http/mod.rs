//! HTTP transport surface for KalamDB.
//!
//! This module groups REST endpoints and HTTP-only helpers by transport
//! instead of keeping them under a generic handlers bucket.

use kalamdb_commons::constants::MAX_SQL_QUERY_LENGTH;

/// JSON body ceiling for Actix's `JsonConfig`.
///
/// A query at [`MAX_SQL_QUERY_LENGTH`] is wrapped in a JSON object that also
/// carries parameters, so the extractor limit is twice the SQL string cap.
pub const HTTP_JSON_BODY_LIMIT: usize = MAX_SQL_QUERY_LENGTH * 2;

/// Largest request body Actix will accept.
///
/// Multipart SQL uploads and table-import zips both stop at this size.
pub const HTTP_PAYLOAD_BODY_LIMIT: usize = 100 * 1024 * 1024;

pub mod auth;
pub mod client_cancel;
pub mod cluster;
pub mod files;
pub mod functions;
pub mod functions_admin;
pub mod sql;
pub mod table_transfer;
pub mod topics;

mod health;

pub(crate) use health::healthcheck_handler;
