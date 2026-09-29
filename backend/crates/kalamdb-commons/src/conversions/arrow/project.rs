//! Nested Arrow projection that shares value buffers and applies parent nulls.

use std::sync::Arc;

use arrow::{
    array::{Array, ArrayRef, BooleanBufferBuilder, LargeListArray, ListArray, StructArray},
    buffer::NullBuffer,
    datatypes::{DataType, Field, Fields},
    record_batch::RecordBatch,
};

/// Project a struct child while sharing the child's value buffers.
///
/// Parent nulls are unioned onto the child validity bitmap so a null parent
/// cannot leak nested values. Value buffers are not copied.
pub fn get_field(batch: &RecordBatch, column: &str, nested: &str) -> Result<ArrayRef, String> {
    let array = batch
        .column_by_name(column)
        .ok_or_else(|| format!("column '{column}' not found"))?;
    get_struct_field(array, nested)
}

/// Project one named child from a `StructArray`, applying parent nulls.
pub fn get_struct_field(array: &dyn Array, nested: &str) -> Result<ArrayRef, String> {
    let struct_array = array
        .as_any()
        .downcast_ref::<StructArray>()
        .ok_or_else(|| format!("column is {:?}, expected Struct", array.data_type()))?;
    let (index, _) = struct_array
        .fields()
        .find(nested)
        .ok_or_else(|| format!("struct field '{nested}' not found"))?;
    let child = struct_array.column(index);
    Ok(apply_parent_nulls(Arc::clone(child), struct_array.nulls()))
}

fn apply_parent_nulls(child: ArrayRef, parent_nulls: Option<&NullBuffer>) -> ArrayRef {
    let Some(parent) = parent_nulls else {
        return child;
    };
    let combined = match child.nulls() {
        Some(child_nulls) => NullBuffer::union(Some(child_nulls), Some(parent)),
        None => {
            let mut builder = BooleanBufferBuilder::new(parent.len());
            for index in 0..parent.len() {
                builder.append(parent.is_valid(index));
            }
            Some(NullBuffer::new(builder.finish()))
        },
    };
    let Some(combined) = combined else {
        return child;
    };
    let data = child.to_data();
    let mut builder = arrow::array::ArrayData::builder(child.data_type().clone())
        .len(data.len())
        .nulls(Some(combined))
        .offset(data.offset());
    for buffer in data.buffers() {
        builder = builder.add_buffer(buffer.clone());
    }
    for child_data in data.child_data() {
        builder = builder.add_child_data(child_data.clone());
    }
    match builder.build() {
        Ok(data) => arrow::array::make_array(data),
        Err(_) => child,
    }
}

/// True when `data_type` is a named-type Arrow layout.
pub fn is_nested_arrow(data_type: &DataType) -> bool {
    matches!(data_type, DataType::Struct(_) | DataType::List(_) | DataType::LargeList(_))
}

/// Rebuild `array` so its `DataType` matches `field`, including nested Field
/// metadata and nullability. Value buffers are reused; missing struct children
/// become null arrays so evolved types can be scanned.
pub fn align_array_to_field(array: ArrayRef, field: &Field) -> Result<ArrayRef, String> {
    align_array_to_type(array, field.data_type())
}

fn align_array_to_type(array: ArrayRef, data_type: &DataType) -> Result<ArrayRef, String> {
    if array.data_type() == data_type {
        return Ok(array);
    }
    if matches!(array.data_type(), DataType::Null) {
        return Ok(arrow::array::new_null_array(data_type, array.len()));
    }
    match data_type {
        DataType::Struct(fields) => align_struct(array, fields),
        DataType::List(item) => align_list(array, item),
        DataType::LargeList(item) => align_large_list(array, item),
        other => arrow::compute::kernels::cast::cast(array.as_ref(), other)
            .map_err(|error| error.to_string()),
    }
}

fn align_struct(array: ArrayRef, fields: &Fields) -> Result<ArrayRef, String> {
    let struct_array = array
        .as_any()
        .downcast_ref::<StructArray>()
        .ok_or_else(|| format!("expected Struct, got {:?}", array.data_type()))?;
    let mut children = Vec::with_capacity(fields.len());
    for field in fields {
        let child = match struct_array.column_by_name(field.name()) {
            Some(column) => align_array_to_type(Arc::clone(column), field.data_type())?,
            None => arrow::array::new_null_array(field.data_type(), struct_array.len()),
        };
        children.push(child);
    }
    Ok(Arc::new(
        StructArray::try_new(fields.clone(), children, struct_array.nulls().cloned())
            .map_err(|error| error.to_string())?,
    ))
}

fn align_list(array: ArrayRef, item: &Arc<Field>) -> Result<ArrayRef, String> {
    let list = array
        .as_any()
        .downcast_ref::<ListArray>()
        .ok_or_else(|| format!("expected List, got {:?}", array.data_type()))?;
    let values = align_array_to_type(Arc::clone(list.values()), item.data_type())?;
    Ok(Arc::new(
        ListArray::try_new(Arc::clone(item), list.offsets().clone(), values, list.nulls().cloned())
            .map_err(|error| error.to_string())?,
    ))
}

fn align_large_list(array: ArrayRef, item: &Arc<Field>) -> Result<ArrayRef, String> {
    let list = array
        .as_any()
        .downcast_ref::<LargeListArray>()
        .ok_or_else(|| format!("expected LargeList, got {:?}", array.data_type()))?;
    let values = align_array_to_type(Arc::clone(list.values()), item.data_type())?;
    Ok(Arc::new(
        LargeListArray::try_new(
            Arc::clone(item),
            list.offsets().clone(),
            values,
            list.nulls().cloned(),
        )
        .map_err(|error| error.to_string())?,
    ))
}

#[cfg(test)]
mod tests {
    use arrow::{
        array::{Int32Array, StringArray, StructArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    };

    use super::*;

    #[test]
    fn get_field_shares_child_buffers_and_applies_parent_nulls() {
        let ids = Int32Array::from(vec![Some(1), Some(2), Some(3)]);
        let cities = StringArray::from(vec![Some("a"), Some("b"), Some("c")]);
        let struct_array = StructArray::new(
            vec![
                Field::new("id", DataType::Int32, true),
                Field::new("city", DataType::Utf8, true),
            ]
            .into(),
            vec![Arc::new(ids) as ArrayRef, Arc::new(cities) as ArrayRef],
            Some(NullBuffer::from(vec![true, false, true])),
        );
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new(
                "address",
                DataType::Struct(struct_array.fields().clone()),
                true,
            )])),
            vec![Arc::new(struct_array) as ArrayRef],
        )
        .unwrap();
        let projected = get_field(&batch, "address", "city").unwrap();
        assert_eq!(projected.len(), 3);
        assert!(projected.is_null(1));
        assert!(!projected.is_null(0));
    }

    #[test]
    fn align_struct_fills_missing_children_and_nested_metadata() {
        let cities = StringArray::from(vec![Some("Paris")]);
        let source = StructArray::from(vec![(
            Arc::new(Field::new("city", DataType::Utf8, true)),
            Arc::new(cities) as ArrayRef,
        )]);
        let mut metadata = std::collections::HashMap::new();
        metadata.insert("PARQUET:field_id".to_string(), "2001".to_string());
        let target = Field::new(
            "home",
            DataType::Struct(
                vec![
                    Field::new("city", DataType::Utf8, true).with_metadata(metadata),
                    Field::new("zip", DataType::Utf8, true),
                ]
                .into(),
            ),
            true,
        );
        let aligned = align_array_to_field(Arc::new(source), &target).unwrap();
        assert_eq!(aligned.data_type(), target.data_type());
        let aligned = aligned.as_any().downcast_ref::<StructArray>().unwrap();
        assert_eq!(aligned.num_columns(), 2);
        assert_eq!(aligned.column(1).null_count(), 1);
    }
}
