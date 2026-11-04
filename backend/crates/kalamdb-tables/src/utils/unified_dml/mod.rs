//! Primary-key validation shared by user and shared table writes.

mod validate;

pub use validate::{extract_user_pk_value, validate_primary_key};
