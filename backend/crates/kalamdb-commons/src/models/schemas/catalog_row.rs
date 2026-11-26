//! Row-to-batch conversion for virtual catalog views.
//!
//! `#[table(view = true)]` emits an implementation. `()` is the empty extension
//! used when an `information_schema` relation adds no columns.

use std::sync::Arc;

use arrow::datatypes::Schema;
use arrow::record_batch::RecordBatch;

/// A catalog row that can be packed into one Arrow batch.
pub trait CatalogRow: Sized + Send + Sync + 'static {
    /// Schema of [`Self::to_batch`].
    fn batch_schema() -> Arc<Schema>;

    /// Build a batch whose row count equals `rows.len()`.
    fn to_batch(rows: &[Self]) -> RecordBatch;
}

impl CatalogRow for () {
    fn batch_schema() -> Arc<Schema> {
        Arc::new(Schema::empty())
    }

    fn to_batch(_rows: &[Self]) -> RecordBatch {
        RecordBatch::new_empty(Self::batch_schema())
    }
}
