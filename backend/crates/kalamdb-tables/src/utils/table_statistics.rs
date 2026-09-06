//! Manifest-backed [`Statistics`] for DataFusion [`TableProvider`] implementations.
//!
//! Aggregates committed cold-segment metadata from manifests. Counts exclude the
//! hot-store tail and MVCC deduplication, so row counts are [`Precision::Inexact`].
//!
//! KalamDB already uses segment `column_stats` for scan pruning; this module
//! exposes table-level aggregates through `TableProvider::statistics` for future
//! optimizer rules. DataFusion's mainline planner does not read that hook today.

use std::collections::HashMap;

use datafusion::{
    arrow::datatypes::SchemaRef,
    common::stats::Precision,
    physical_plan::{ColumnStatistics, Statistics},
    scalar::ScalarValue,
};
use kalamdb_commons::{
    models::{
        rows::{choose_max_stored_scalar, choose_min_stored_scalar, StoredScalarValue},
        schemas::TableDefinition,
    },
    schemas::TableType,
    TableId, UserId,
};
use kalamdb_system::{
    ColumnStats, Manifest, ManifestService as ManifestServiceTrait, SegmentMetadata,
};

use crate::utils::core::TableProviderCore;

/// Build table statistics from manifest segment metadata.
///
/// Returns `None` when manifests cannot be loaded. When no cold segments exist,
/// returns zero-row statistics with unknown per-column bounds.
pub fn compute_manifest_table_statistics(
    core: &TableProviderCore,
    table_type: TableType,
) -> Option<Statistics> {
    let schema = core.schema().clone();
    let table_id = core.table_id();
    let manifest_service = core.manifest_service();

    let manifests = load_manifests(manifest_service.as_ref(), table_id, table_type)?;
    Some(aggregate_manifest_statistics(&schema, core.table_def(), &manifests))
}

fn load_manifests(
    manifest_service: &dyn ManifestServiceTrait,
    table_id: &TableId,
    table_type: TableType,
) -> Option<Vec<Manifest>> {
    match table_type {
        TableType::User => {
            let user_ids = manifest_service.get_manifest_user_ids(table_id).ok()?;
            if user_ids.is_empty() {
                return load_manifest(manifest_service, table_id, None)
                    .map(|manifest| vec![manifest]);
            }

            let manifests: Vec<Manifest> = user_ids
                .iter()
                .filter_map(|user_id| load_manifest(manifest_service, table_id, Some(user_id)))
                .collect();
            Some(manifests)
        },
        TableType::Shared | TableType::Stream => {
            load_manifest(manifest_service, table_id, None).map(|manifest| vec![manifest])
        },
        _ => None,
    }
}

fn load_manifest(
    manifest_service: &dyn ManifestServiceTrait,
    table_id: &TableId,
    user_id: Option<&UserId>,
) -> Option<Manifest> {
    manifest_service
        .get_or_load(table_id, user_id)
        .ok()
        .flatten()
        .map(|entry| entry.manifest.clone())
}

fn aggregate_manifest_statistics(
    schema: &SchemaRef,
    table_def: &TableDefinition,
    manifests: &[Manifest],
) -> Statistics {
    let readable_segments: Vec<&SegmentMetadata> = manifests
        .iter()
        .flat_map(|manifest| manifest.segments.iter())
        .filter(|segment| segment.status.is_readable())
        .collect();

    if readable_segments.is_empty() {
        return empty_statistics(schema);
    }

    let num_rows = readable_segments.iter().map(|segment| segment.row_count).sum::<u64>() as usize;
    let total_byte_size =
        readable_segments.iter().map(|segment| segment.size_bytes).sum::<u64>() as usize;

    let column_id_to_index = column_id_index_map(table_def, schema);
    let mut merged_stats: HashMap<usize, MergedColumnStats> = HashMap::new();

    for segment in readable_segments {
        for (column_id, stats) in &segment.column_stats {
            let Some(column_index) = column_id_to_index.get(column_id).copied() else {
                continue;
            };
            merged_stats.entry(column_index).or_default().merge(stats, segment.row_count);
        }
    }

    let column_statistics = (0..schema.fields().len())
        .map(|index| {
            merged_stats
                .get(&index)
                .map(MergedColumnStats::to_column_statistics)
                .unwrap_or_else(ColumnStatistics::new_unknown)
        })
        .collect();

    Statistics {
        // Cold segments omit hot tail rows and MVCC deduplication effects.
        num_rows: Precision::Inexact(num_rows),
        total_byte_size: Precision::Inexact(total_byte_size),
        column_statistics,
    }
}

fn empty_statistics(schema: &SchemaRef) -> Statistics {
    Statistics {
        num_rows:          Precision::Exact(0),
        total_byte_size:   Precision::Exact(0),
        column_statistics: Statistics::unknown_column(schema),
    }
}

fn column_id_index_map(table_def: &TableDefinition, schema: &SchemaRef) -> HashMap<u64, usize> {
    let name_to_index: HashMap<&str, usize> = schema
        .fields()
        .iter()
        .enumerate()
        .map(|(index, field)| (field.name().as_str(), index))
        .collect();

    table_def
        .columns
        .iter()
        .filter_map(|column| {
            name_to_index
                .get(column.column_name.as_str())
                .map(|index| (column.column_id, *index))
        })
        .collect()
}

#[derive(Default)]
struct MergedColumnStats {
    min_value:  Option<StoredScalarValue>,
    max_value:  Option<StoredScalarValue>,
    null_count: u64,
}

impl MergedColumnStats {
    fn merge(&mut self, stats: &ColumnStats, segment_rows: u64) {
        self.min_value = choose_min_stored_scalar(self.min_value.take(), stats.min.clone());
        self.max_value = choose_max_stored_scalar(self.max_value.take(), stats.max.clone());
        if let Some(null_count) = stats.null_count {
            self.null_count = self.null_count.saturating_add(null_count.max(0) as u64);
        } else if stats.min.is_none() && stats.max.is_none() && segment_rows > 0 {
            // Segment has rows but no typed bounds; leave null_count unchanged.
        }
    }

    fn to_column_statistics(&self) -> ColumnStatistics {
        ColumnStatistics {
            null_count:     if self.null_count > 0 {
                Precision::Inexact(self.null_count as usize)
            } else {
                Precision::Exact(0)
            },
            min_value:      stored_scalar_precision(self.min_value.clone()),
            max_value:      stored_scalar_precision(self.max_value.clone()),
            sum_value:      Precision::Absent,
            distinct_count: Precision::Absent,
            byte_size:      Precision::Absent,
        }
    }
}

fn stored_scalar_precision(value: Option<StoredScalarValue>) -> Precision<ScalarValue> {
    value.map(ScalarValue::from).map(Precision::Exact).unwrap_or(Precision::Absent)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use datafusion::arrow::datatypes::{DataType, Field, Schema};
    use kalamdb_commons::{
        datatypes::KalamDataType,
        models::schemas::{ColumnDefinition, TableDefinition, TableOptions},
        NamespaceId, TableName,
    };

    use super::*;

    fn test_table_def() -> TableDefinition {
        TableDefinition::new(
            NamespaceId::new("app"),
            TableName::new("items"),
            TableType::Shared,
            vec![
                ColumnDefinition::new(
                    1,
                    "id",
                    1,
                    KalamDataType::BigInt,
                    false,
                    true,
                    false,
                    kalamdb_commons::schemas::ColumnDefault::None,
                    None,
                ),
                ColumnDefinition::new(
                    2,
                    "score",
                    2,
                    KalamDataType::Int,
                    true,
                    false,
                    false,
                    kalamdb_commons::schemas::ColumnDefault::None,
                    None,
                ),
            ],
            TableOptions::shared(),
            None,
        )
        .expect("table definition")
    }

    fn test_schema() -> SchemaRef {
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("score", DataType::Int32, true),
        ]))
    }

    fn test_segment(
        row_count: u64,
        id_min: i64,
        id_max: i64,
        score_min: i32,
        score_max: i32,
    ) -> SegmentMetadata {
        let mut column_stats = HashMap::new();
        column_stats.insert(
            1,
            ColumnStats::new(
                Some(StoredScalarValue::Int64(Some(id_min.to_string()))),
                Some(StoredScalarValue::Int64(Some(id_max.to_string()))),
                Some(0),
            ),
        );
        column_stats.insert(
            2,
            ColumnStats::new(
                Some(StoredScalarValue::Int32(Some(score_min))),
                Some(StoredScalarValue::Int32(Some(score_max))),
                Some(1),
            ),
        );

        SegmentMetadata {
            min_seq: kalamdb_commons::ids::SeqId::from(1),
            max_seq: kalamdb_commons::ids::SeqId::from(row_count as i64),
            row_count,
            size_bytes: row_count * 64,
            created_at: 0,
            id: format!("seg-{row_count}"),
            path: format!("batch-{row_count}.parquet"),
            column_stats,
            schema_version: 1,
            status: kalamdb_system::SegmentStatus::Committed,
        }
    }

    #[test]
    fn aggregate_manifest_statistics_sums_cold_segments() {
        let schema = test_schema();
        let table_def = test_table_def();
        let manifest = Manifest::new(TableId::from_strings("app", "items"), None);
        let mut manifest = manifest;
        manifest.add_segment(test_segment(100, 1, 100, 10, 90));
        manifest.add_segment(test_segment(50, 101, 150, 5, 95));

        let stats = aggregate_manifest_statistics(&schema, &table_def, &[manifest]);

        assert_eq!(stats.num_rows, Precision::Inexact(150));
        assert_eq!(stats.total_byte_size, Precision::Inexact(150 * 64));
        assert_eq!(stats.column_statistics.len(), 2);
        assert_eq!(
            stats.column_statistics[0].min_value,
            Precision::Exact(ScalarValue::Int64(Some(1)))
        );
        assert_eq!(
            stats.column_statistics[0].max_value,
            Precision::Exact(ScalarValue::Int64(Some(150)))
        );
        assert_eq!(stats.column_statistics[1].null_count, Precision::Inexact(2));
    }

    #[test]
    fn empty_manifest_returns_zero_row_statistics() {
        let schema = test_schema();
        let table_def = test_table_def();
        let manifest = Manifest::new(TableId::from_strings("app", "items"), None);

        let stats = aggregate_manifest_statistics(&schema, &table_def, &[manifest]);

        assert_eq!(stats.num_rows, Precision::Exact(0));
        assert_eq!(stats.total_byte_size, Precision::Exact(0));
    }
}
