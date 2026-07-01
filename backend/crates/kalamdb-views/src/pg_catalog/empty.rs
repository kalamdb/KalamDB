use std::sync::Arc;

use datafusion::arrow::{
    array::{ArrayRef, BooleanArray, Float64Array, Int64Array, StringArray},
    datatypes::{DataType, Field, Schema, SchemaRef},
    record_batch::RecordBatch,
};
use kalamdb_commons::Role;

use crate::{error::RegistryError, pg_catalog::PgCatalogView};

#[derive(Debug)]
pub struct EmptyPgCatalogView {
    name: &'static str,
    schema: SchemaRef,
}

impl EmptyPgCatalogView {
    pub fn new(name: &'static str, fields: Vec<Field>) -> Self {
        Self {
            name,
            schema: Arc::new(Schema::new(fields)),
        }
    }
}

impl PgCatalogView for EmptyPgCatalogView {
    fn name(&self) -> &'static str {
        self.name
    }

    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }

    fn compute_batch(&self, _role: Role) -> Result<RecordBatch, RegistryError> {
        let arrays = self
            .schema
            .fields()
            .iter()
            .map(|field| empty_array_for_field(field.as_ref()))
            .collect::<Vec<_>>();
        RecordBatch::try_new(Arc::clone(&self.schema), arrays).map_err(|error| {
            RegistryError::Other(format!("failed to build pg_catalog.{}: {error}", self.name))
        })
    }
}

fn empty_array_for_field(field: &Field) -> ArrayRef {
    match field.data_type() {
        DataType::Boolean => Arc::new(BooleanArray::from(Vec::<bool>::new())) as ArrayRef,
        DataType::Float64 => Arc::new(Float64Array::from(Vec::<f64>::new())) as ArrayRef,
        DataType::Int64 => Arc::new(Int64Array::from(Vec::<i64>::new())) as ArrayRef,
        DataType::Utf8 => Arc::new(StringArray::from(Vec::<String>::new())) as ArrayRef,
        _ => Arc::new(StringArray::from(Vec::<String>::new())) as ArrayRef,
    }
}
