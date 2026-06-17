//! Partition [`PlanProperties`] builders for shared execution plans.

use datafusion::{
    physical_expr::EquivalenceProperties,
    physical_plan::{
        execution_plan::{Boundedness, EmissionType},
        Partitioning, PlanProperties,
    },
};

/// Build conservative [`PlanProperties`] for a single-partition, bounded
/// source. Sources with richer guarantees should call [`PlanProperties::new`]
/// directly with their own equivalence properties.
pub fn single_partition_plan_properties(schema: arrow_schema::SchemaRef) -> PlanProperties {
    PlanProperties::new(
        EquivalenceProperties::new(schema),
        Partitioning::UnknownPartitioning(1),
        EmissionType::Incremental,
        Boundedness::Bounded,
    )
}

/// Build [`PlanProperties`] for a bounded source that emits a known number of
/// independent partitions (for example, one per Parquet segment).
pub fn multi_partition_plan_properties(
    schema: arrow_schema::SchemaRef,
    partition_count: usize,
) -> PlanProperties {
    let partitioning = if partition_count <= 1 {
        Partitioning::UnknownPartitioning(1)
    } else {
        Partitioning::UnknownPartitioning(partition_count)
    };

    PlanProperties::new(
        EquivalenceProperties::new(schema),
        partitioning,
        EmissionType::Incremental,
        Boundedness::Bounded,
    )
}
