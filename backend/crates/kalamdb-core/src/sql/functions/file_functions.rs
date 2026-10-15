//! Built-in KalamDB file extension scalar functions (`file_put`, `file_read`, `file_delete`).

use std::{
    hash::{Hash, Hasher},
    sync::Arc,
};

use datafusion::{
    arrow::array::{Array, ArrayRef, BinaryBuilder, StringArray, StringBuilder},
    error::{DataFusionError, Result as DataFusionResult},
    logical_expr::{ColumnarValue, ScalarFunctionArgs, ScalarUDFImpl, Signature},
    scalar::ScalarValue,
};
use kalamdb_commons::{
    arrow_utils::{arrow_utf8, ArrowDataType},
    FileRef,
};
use kalamdb_filestore::FileStorageService;

use super::scalar::{any_one_volatile, cached_signature};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum FileFunctionKind {
    Put,
    Read,
    Delete,
}

/// ScalarUDF implementation for `file_put(placeholder)`, `file_read(file)`, and
/// `file_delete(file)`.
#[derive(Clone)]
pub struct FileStorageFunction {
    name:    &'static str,
    kind:    FileFunctionKind,
    service: Option<Arc<FileStorageService>>,
}

impl std::fmt::Debug for FileStorageFunction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileStorageFunction")
            .field("name", &self.name)
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

impl PartialEq for FileStorageFunction {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.kind == other.kind
    }
}

impl Eq for FileStorageFunction {}

impl Hash for FileStorageFunction {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.kind.hash(state);
    }
}

impl FileStorageFunction {
    pub fn file_put(service: Option<Arc<FileStorageService>>) -> Self {
        Self {
            name: "file_put",
            kind: FileFunctionKind::Put,
            service,
        }
    }

    pub fn file_read(service: Option<Arc<FileStorageService>>) -> Self {
        Self {
            name: "file_read",
            kind: FileFunctionKind::Read,
            service,
        }
    }

    pub fn file_delete(service: Option<Arc<FileStorageService>>) -> Self {
        Self {
            name: "file_delete",
            kind: FileFunctionKind::Delete,
            service,
        }
    }

    fn require_service(&self) -> DataFusionResult<&Arc<FileStorageService>> {
        self.service.as_ref().ok_or_else(|| {
            DataFusionError::Execution(format!("{} is not initialized on this session", self.name))
        })
    }
}

impl ScalarUDFImpl for FileStorageFunction {
    fn name(&self) -> &str {
        self.name
    }

    fn signature(&self) -> &Signature {
        cached_signature!(any_one_volatile())
    }

    fn return_type(&self, _arg_types: &[ArrowDataType]) -> DataFusionResult<ArrowDataType> {
        match self.kind {
            FileFunctionKind::Put => Ok(arrow_utf8()),
            FileFunctionKind::Read => Ok(ArrowDataType::Binary),
            FileFunctionKind::Delete => Ok(arrow_utf8()),
        }
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DataFusionResult<ColumnarValue> {
        if args.args.len() != 1 {
            return Err(DataFusionError::Plan(format!(
                "{}() expects exactly 1 argument, got {}",
                self.name,
                args.args.len()
            )));
        }
        let service = self.require_service()?;
        match &args.args[0] {
            ColumnarValue::Scalar(scalar) => invoke_scalar(self.kind, service, scalar),
            ColumnarValue::Array(array) => invoke_array(self.kind, service, array),
        }
    }
}

fn scalar_as_str(scalar: &ScalarValue) -> DataFusionResult<Option<String>> {
    match scalar {
        ScalarValue::Null
        | ScalarValue::Utf8(None)
        | ScalarValue::LargeUtf8(None)
        | ScalarValue::Utf8View(None) => Ok(None),
        ScalarValue::Utf8(Some(value))
        | ScalarValue::LargeUtf8(Some(value))
        | ScalarValue::Utf8View(Some(value)) => Ok(Some(value.clone())),
        other => Err(DataFusionError::Execution(format!(
            "expected text/file value, got {}",
            other.data_type()
        ))),
    }
}

fn parse_file_ref(raw: &str, fn_name: &str) -> DataFusionResult<FileRef> {
    let file_ref = FileRef::try_from_json(raw).map_err(|error| {
        DataFusionError::Execution(format!("invalid pg_catalog.file value for {fn_name}: {error}"))
    })?;
    if !file_ref.is_well_formed() {
        return Err(DataFusionError::Execution(format!(
            "invalid pg_catalog.file value for {fn_name}: required fields missing or malformed"
        )));
    }
    Ok(file_ref)
}

fn invoke_scalar(
    kind: FileFunctionKind,
    service: &Arc<FileStorageService>,
    scalar: &ScalarValue,
) -> DataFusionResult<ColumnarValue> {
    let Some(raw) = scalar_as_str(scalar)? else {
        return Ok(ColumnarValue::Scalar(match kind {
            FileFunctionKind::Put | FileFunctionKind::Delete => ScalarValue::Utf8(None),
            FileFunctionKind::Read => ScalarValue::Binary(None),
        }));
    };

    match kind {
        FileFunctionKind::Put => {
            let file_ref = service
                .resolve_placeholder(&raw)
                .map_err(|error| DataFusionError::Execution(error.to_string()))?;
            Ok(ColumnarValue::Scalar(ScalarValue::Utf8(Some(file_ref.to_json()))))
        },
        FileFunctionKind::Read => {
            let file_ref = parse_file_ref(&raw, "file_read")?;
            let bytes = service
                .read_file_ref_sync(&file_ref)
                .map_err(|error| DataFusionError::Execution(error.to_string()))?;
            Ok(ColumnarValue::Scalar(ScalarValue::Binary(Some(bytes.to_vec()))))
        },
        FileFunctionKind::Delete => {
            let file_ref = parse_file_ref(&raw, "file_delete")?;
            service
                .delete_file_ref_sync(&file_ref)
                .map_err(|error| DataFusionError::Execution(error.to_string()))?;
            Ok(ColumnarValue::Scalar(ScalarValue::Utf8(None)))
        },
    }
}

fn invoke_array(
    kind: FileFunctionKind,
    service: &Arc<FileStorageService>,
    array: &ArrayRef,
) -> DataFusionResult<ColumnarValue> {
    let string_array = array.as_any().downcast_ref::<StringArray>().ok_or_else(|| {
        DataFusionError::Execution(format!("expected Utf8 array, got {:?}", array.data_type()))
    })?;

    match kind {
        FileFunctionKind::Put => {
            let mut builder = StringBuilder::with_capacity(string_array.len(), 128);
            for idx in 0..string_array.len() {
                if string_array.is_null(idx) {
                    builder.append_null();
                } else {
                    let file_ref = service
                        .resolve_placeholder(string_array.value(idx))
                        .map_err(|error| DataFusionError::Execution(error.to_string()))?;
                    builder.append_value(file_ref.to_json());
                }
            }
            Ok(ColumnarValue::Array(Arc::new(builder.finish())))
        },
        FileFunctionKind::Read => {
            let mut builder = BinaryBuilder::with_capacity(string_array.len(), 256);
            for idx in 0..string_array.len() {
                if string_array.is_null(idx) {
                    builder.append_null();
                } else {
                    let file_ref = parse_file_ref(string_array.value(idx), "file_read")?;
                    let bytes = service
                        .read_file_ref_sync(&file_ref)
                        .map_err(|error| DataFusionError::Execution(error.to_string()))?;
                    builder.append_value(bytes.as_ref());
                }
            }
            Ok(ColumnarValue::Array(Arc::new(builder.finish())))
        },
        FileFunctionKind::Delete => {
            let mut builder = StringBuilder::with_capacity(string_array.len(), 0);
            for idx in 0..string_array.len() {
                if !string_array.is_null(idx) {
                    let file_ref = parse_file_ref(string_array.value(idx), "file_delete")?;
                    service
                        .delete_file_ref_sync(&file_ref)
                        .map_err(|error| DataFusionError::Execution(error.to_string()))?;
                }
                builder.append_null();
            }
            Ok(ColumnarValue::Array(Arc::new(builder.finish())))
        },
    }
}
