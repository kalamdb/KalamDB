//! Merges a transaction overlay with committed scan batches.

mod merge;
mod stream;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use datafusion::{
    common::{tree_node::TreeNodeRecursion, Result as DataFusionResult},
    error::DataFusionError,
    execution::{SendableRecordBatchStream, TaskContext},
    physical_expr::{EquivalenceProperties, PhysicalExpr},
    physical_plan::{
        ChildrenPropertiesMode, DisplayAs, DisplayFormatType, ExecutionPlan,
        ExecutionPlanProperties, Partitioning, PlanProperties, ReplaceChildrenOptions,
    },
};
use kalamdb_commons::{models::UserId, TableId};
use kalamdb_datafusion_sources::exec::projected_schema;

use crate::query_context::TransactionOverlayView;

use stream::OverlayMergeStream;

/// Physical execution node that merges transaction-local overlay rows with committed scan output.
#[derive(Debug, Clone)]
pub struct TransactionOverlayExec {
    input: Arc<dyn ExecutionPlan>,
    table_id: TableId,
    primary_key_column: Arc<str>,
    /// Read when the plan executes, so a live plan holds no overlay snapshot.
    overlay_view: Arc<dyn TransactionOverlayView>,
    user_scope: Option<UserId>,
    final_projection: Option<Vec<usize>>,
    fetch: Option<usize>,
    /// Predicates the child scan already applied to committed rows. Overlay
    /// rows are merged after that scan, so the same predicate has to run on
    /// the merged batch. Otherwise `NOT IN` and `<>` keep uncommitted rows
    /// that do not match.
    residual_filter: Option<Arc<dyn PhysicalExpr>>,
    cache: Arc<PlanProperties>,
}

impl TransactionOverlayExec {
    pub fn try_new(
        input: Arc<dyn ExecutionPlan>,
        table_id: TableId,
        primary_key_column: impl Into<Arc<str>>,
        overlay_view: Arc<dyn TransactionOverlayView>,
        user_scope: Option<UserId>,
        final_projection: Option<Vec<usize>>,
        fetch: Option<usize>,
        residual_filter: Option<Arc<dyn PhysicalExpr>>,
    ) -> DataFusionResult<Self> {
        let output_schema = projected_schema(&input.schema(), final_projection.as_deref())?;
        let cache = Arc::new(PlanProperties::new(
            EquivalenceProperties::new(Arc::clone(&output_schema)),
            // Overlay rows apply to the whole relation, so this node must collapse
            // child partitions and emit a single merged output partition.
            Partitioning::UnknownPartitioning(1),
            input.pipeline_behavior(),
            input.boundedness(),
        ));

        Ok(Self {
            input,
            table_id,
            primary_key_column: primary_key_column.into(),
            overlay_view,
            user_scope,
            final_projection,
            fetch,
            residual_filter,
            cache,
        })
    }
}

impl DisplayAs for TransactionOverlayExec {
    fn fmt_as(&self, t: DisplayFormatType, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match t {
            DisplayFormatType::Default | DisplayFormatType::Verbose => {
                write!(
                    f,
                    "TransactionOverlayExec: table={}, pk={}, fetch={:?}",
                    self.table_id, self.primary_key_column, self.fetch
                )
            },
            DisplayFormatType::TreeRender => {
                write!(f, "table={}, pk={}", self.table_id, self.primary_key_column)
            },
        }
    }
}

impl ExecutionPlan for TransactionOverlayExec {
    fn name(&self) -> &str {
        Self::static_name()
    }

    fn properties(&self) -> &Arc<PlanProperties> {
        &self.cache
    }

    fn children(&self) -> Vec<&Arc<dyn ExecutionPlan>> {
        vec![&self.input]
    }

    fn maintains_input_order(&self) -> Vec<bool> {
        vec![false]
    }

    fn apply_expressions(
        &self,
        _f: &mut dyn FnMut(&Arc<dyn PhysicalExpr>) -> DataFusionResult<TreeNodeRecursion>,
    ) -> DataFusionResult<TreeNodeRecursion> {
        Ok(TreeNodeRecursion::Continue)
    }

    fn replace_children(
        self: Arc<Self>,
        mut children: Vec<Arc<dyn ExecutionPlan>>,
        options: ReplaceChildrenOptions,
    ) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
        if children.len() != 1 {
            return Err(DataFusionError::Internal(
                "TransactionOverlayExec expects exactly one child".to_string(),
            ));
        }

        match options.children_properties {
            ChildrenPropertiesMode::Keep => {
                let mut plan = Self::clone(&*self);
                plan.input = children.swap_remove(0);
                Ok(Arc::new(plan))
            },
            ChildrenPropertiesMode::Recompute => Ok(Arc::new(Self::try_new(
                children.swap_remove(0),
                self.table_id.clone(),
                Arc::clone(&self.primary_key_column),
                Arc::clone(&self.overlay_view),
                self.user_scope.clone(),
                self.final_projection.clone(),
                self.fetch,
                self.residual_filter.clone(),
            )?)),
        }
    }

    fn with_new_children(
        self: Arc<Self>,
        children: Vec<Arc<dyn ExecutionPlan>>,
    ) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
        self.replace_children(
            children,
            ReplaceChildrenOptions::new(ChildrenPropertiesMode::Recompute),
        )
    }

    fn execute(
        &self,
        partition: usize,
        context: Arc<TaskContext>,
    ) -> DataFusionResult<SendableRecordBatchStream> {
        if partition != 0 {
            return Err(DataFusionError::Execution(format!(
                "TransactionOverlayExec only supports partition 0, got {partition}",
            )));
        }

        let input = Arc::clone(&self.input);
        let overlay = self.overlay_view.overlay_for_table(&self.table_id);
        let table_id = self.table_id.clone();
        let primary_key_column = Arc::clone(&self.primary_key_column);
        let final_projection = self.final_projection.clone();
        let output_schema = self.schema();
        let input_schema = self.input.schema();
        Ok(Box::pin(OverlayMergeStream::new(
            output_schema,
            input_schema,
            input,
            context,
            table_id,
            primary_key_column,
            overlay,
            self.user_scope.clone(),
            final_projection,
            self.fetch,
            self.residual_filter.clone(),
        )))
    }
}
