//! Statement-scoped UPDATE expressions evaluated over bounded Arrow batches.

use std::{collections::BTreeMap, sync::Arc};

use datafusion::{
    arrow::datatypes::SchemaRef,
    catalog::Session,
    common::DFSchema,
    error::{DataFusionError, Result},
    logical_expr::{ColumnarValue, Expr},
    physical_expr::PhysicalExpr,
};
use kalamdb_commons::{
    conversions::arrow_json_conversion::{arrow_value_to_scalar, json_rows_to_arrow_batch},
    models::rows::Row,
};

pub(crate) struct PreparedUpdateAssignments {
    schema:      SchemaRef,
    expressions: Vec<(String, Arc<dyn PhysicalExpr>)>,
}

impl PreparedUpdateAssignments {
    pub(crate) fn new(
        state: &dyn Session,
        schema: &SchemaRef,
        assignments: &[(String, Expr)],
    ) -> Result<Self> {
        let df_schema = DFSchema::try_from(Arc::clone(schema))?;
        let expressions = assignments
            .iter()
            .map(|(column, expr)| {
                Ok((column.clone(), state.create_physical_expr(expr.clone(), &df_schema)?))
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            schema: Arc::clone(schema),
            expressions,
        })
    }

    pub(crate) fn evaluate(&self, rows: &[Row]) -> Result<Vec<Row>> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let batch = json_rows_to_arrow_batch(&self.schema, rows.to_vec())
            .map_err(DataFusionError::Execution)?;
        let mut updates = vec![Row::new(BTreeMap::new()); rows.len()];
        // Every assignment reads the original batch, including when another
        // assignment targets one of its input columns.
        for (column, expression) in &self.expressions {
            match expression.evaluate(&batch)? {
                ColumnarValue::Scalar(value) => {
                    for row in &mut updates {
                        row.values.insert(column.clone(), value.clone());
                    }
                },
                ColumnarValue::Array(values) => {
                    for (index, row) in updates.iter_mut().enumerate() {
                        row.values
                            .insert(column.clone(), arrow_value_to_scalar(values.as_ref(), index)?);
                    }
                },
            }
        }
        Ok(updates)
    }
}
