//! Performance-optimized macros for building RecordBatch from system table data
//!
//! These macros provide zero-cost abstractions for converting entity data into
//! Arrow RecordBatches with optimal memory usage.

/// Build a RecordBatch from entity data with schema-driven column extraction
///
/// This macro generates optimized code that:
/// - Pre-allocates all array builders based on row count
/// - Iterates data once for all columns (single pass)
/// - Uses schema field order automatically
/// - Zero runtime overhead (compile-time code generation)
///
/// # Example
/// ```rust,ignore
/// build_record_batch!(
///     schema: self.schema.clone(),
///     entries: entries,
///     columns: [
///         cache_key => String(|e| &e.0),
///         namespace_id => String(|e| &e.1),
///         etag => OptionalString(|e| e.4.etag.as_deref()),
///         last_refreshed => Timestamp(|e| Some(e.4.last_refreshed * 1000)),
///         ttl_seconds => Int64(|e| Some(0i64)),
///     ]
/// )
/// ```
#[macro_export]
macro_rules! build_record_batch {
    (
        schema: $schema:expr,
        entries: $entries:expr,
        columns: [
            $($field_name:ident => $field_type:ident($extractor:expr)),+ $(,)?
        ]
    ) => {{
        let row_count = $entries.len();

        // Pre-allocate all builders
        $(
            let mut $field_name = $crate::array_builder!($field_type, row_count);
        )+

        // Single-pass iteration over all entries
        for entry in $entries.iter() {
            $(
                $crate::append_value!($field_name, $field_type, $extractor, entry);
            )+
        }

        // Build RecordBatch with schema-ordered columns
        let columns: Vec<datafusion::arrow::array::ArrayRef> = vec![
            $(
                $crate::finish_array!($field_name, $field_type),
            )+
        ];

        datafusion::arrow::array::RecordBatch::try_new($schema, columns)
    }};
}

/// Helper macro to create array builders with optimal capacity
#[macro_export]
macro_rules! array_builder {
    (String, $row_count:expr, $avg_size:expr) => {
        datafusion::arrow::array::StringBuilder::with_capacity($row_count, $row_count * $avg_size)
    };
    (String, $row_count:expr) => {
        datafusion::arrow::array::StringBuilder::with_capacity($row_count, $row_count * 16)
    };
    (OptionalString, $row_count:expr, $avg_size:expr) => {
        datafusion::arrow::array::StringBuilder::with_capacity($row_count, $row_count * $avg_size)
    };
    (OptionalString, $row_count:expr) => {
        datafusion::arrow::array::StringBuilder::with_capacity($row_count, $row_count * 16)
    };
    (Timestamp, $row_count:expr) => {
        Vec::<Option<i64>>::with_capacity($row_count)
    };
    (OptionalTimestamp, $row_count:expr) => {
        Vec::<Option<i64>>::with_capacity($row_count)
    };
    (Int64, $row_count:expr) => {
        Vec::<Option<i64>>::with_capacity($row_count)
    };
    (OptionalInt64, $row_count:expr) => {
        Vec::<Option<i64>>::with_capacity($row_count)
    };
    (Int32, $row_count:expr) => {
        Vec::<Option<i32>>::with_capacity($row_count)
    };
    (OptionalInt32, $row_count:expr) => {
        Vec::<Option<i32>>::with_capacity($row_count)
    };
    (Boolean, $row_count:expr) => {
        Vec::<Option<bool>>::with_capacity($row_count)
    };
    (OptionalBoolean, $row_count:expr) => {
        Vec::<Option<bool>>::with_capacity($row_count)
    };
}

/// Helper macro to append values to array builders
#[doc(hidden)]
pub fn eval_extractor<'a, E, T, F>(entry: &'a E, extractor: F) -> T
where
    F: Fn(&'a E) -> T,
{
    extractor(entry)
}

/// Helper macro to append values to array builders
#[macro_export]
macro_rules! append_value {
    ($builder:ident, String, $extractor:expr, $entry:expr) => {
        $builder.append_value($crate::macros::eval_extractor($entry, $extractor))
    };
    ($builder:ident, OptionalString, $extractor:expr, $entry:expr) => {
        $builder.append_option($crate::macros::eval_extractor($entry, $extractor))
    };
    ($builder:ident, Timestamp, $extractor:expr, $entry:expr) => {
        $builder.push($crate::macros::eval_extractor($entry, $extractor))
    };
    ($builder:ident, OptionalTimestamp, $extractor:expr, $entry:expr) => {
        $builder.push($crate::macros::eval_extractor($entry, $extractor))
    };
    ($builder:ident, Int64, $extractor:expr, $entry:expr) => {
        $builder.push($crate::macros::eval_extractor($entry, $extractor))
    };
    ($builder:ident, OptionalInt64, $extractor:expr, $entry:expr) => {
        $builder.push($crate::macros::eval_extractor($entry, $extractor))
    };
    ($builder:ident, Int32, $extractor:expr, $entry:expr) => {
        $builder.push($crate::macros::eval_extractor($entry, $extractor))
    };
    ($builder:ident, OptionalInt32, $extractor:expr, $entry:expr) => {
        $builder.push($crate::macros::eval_extractor($entry, $extractor))
    };
    ($builder:ident, Boolean, $extractor:expr, $entry:expr) => {
        $builder.push($crate::macros::eval_extractor($entry, $extractor))
    };
    ($builder:ident, OptionalBoolean, $extractor:expr, $entry:expr) => {
        $builder.push($crate::macros::eval_extractor($entry, $extractor))
    };
}

/// Helper macro to finish array builders and cast to ArrayRef
#[macro_export]
macro_rules! finish_array {
    ($builder:ident, String) => {
        std::sync::Arc::new($builder.finish()) as datafusion::arrow::array::ArrayRef
    };
    ($builder:ident, OptionalString) => {
        std::sync::Arc::new($builder.finish()) as datafusion::arrow::array::ArrayRef
    };
    ($builder:ident, Timestamp) => {
        std::sync::Arc::new(datafusion::arrow::array::TimestampMicrosecondArray::from(
            $builder.into_iter().map(|ts| ts.map(|ms| ms * 1000)).collect::<Vec<_>>(),
        )) as datafusion::arrow::array::ArrayRef
    };
    ($builder:ident, OptionalTimestamp) => {
        std::sync::Arc::new(datafusion::arrow::array::TimestampMicrosecondArray::from(
            $builder.into_iter().map(|ts| ts.map(|ms| ms * 1000)).collect::<Vec<_>>(),
        )) as datafusion::arrow::array::ArrayRef
    };
    ($builder:ident, Int64) => {
        std::sync::Arc::new(datafusion::arrow::array::Int64Array::from($builder))
            as datafusion::arrow::array::ArrayRef
    };
    ($builder:ident, OptionalInt64) => {
        std::sync::Arc::new(datafusion::arrow::array::Int64Array::from($builder))
            as datafusion::arrow::array::ArrayRef
    };
    ($builder:ident, Int32) => {
        std::sync::Arc::new(datafusion::arrow::array::Int32Array::from($builder))
            as datafusion::arrow::array::ArrayRef
    };
    ($builder:ident, OptionalInt32) => {
        std::sync::Arc::new(datafusion::arrow::array::Int32Array::from($builder))
            as datafusion::arrow::array::ArrayRef
    };
    ($builder:ident, Boolean) => {
        std::sync::Arc::new(datafusion::arrow::array::BooleanArray::from($builder))
            as datafusion::arrow::array::ArrayRef
    };
    ($builder:ident, OptionalBoolean) => {
        std::sync::Arc::new(datafusion::arrow::array::BooleanArray::from($builder))
            as datafusion::arrow::array::ArrayRef
    };
}
