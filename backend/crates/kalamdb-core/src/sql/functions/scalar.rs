//! Shared `ScalarUDFImpl` shell and column helpers for SQL functions.
//!
//! DataFusion's `ScalarUDFImpl` is a foreign trait, so a blanket impl over a local
//! marker trait is rejected by the orphan rules. `scalar_udf!` generates the unit
//! struct and the methods that are the same for every stateless function. Functions
//! that carry state keep their own impl and call the signature and column helpers.

use std::sync::Arc;

use datafusion::{
    arrow::array::{BooleanBuilder, StringArray, StringBuilder},
    error::{DataFusionError, Result as DataFusionResult},
    logical_expr::{ColumnarValue, ScalarFunctionArgs, Signature, TypeSignature, Volatility},
};
use kalamdb_commons::arrow_utils::{arrow_utf8, ArrowDataType};

pub(crate) fn utf8_type() -> ArrowDataType {
    arrow_utf8()
}

pub(crate) fn int32_type() -> ArrowDataType {
    ArrowDataType::Int32
}

pub(crate) fn zero_arg_stable() -> Signature {
    Signature::exact(vec![], Volatility::Stable)
}

pub(crate) fn zero_arg_volatile() -> Signature {
    Signature::exact(vec![], Volatility::Volatile)
}

pub(crate) fn one_integer_stable() -> Signature {
    stable_exact(&[
        &[ArrowDataType::Int64],
        &[ArrowDataType::Int32],
        &[ArrowDataType::UInt64],
    ])
}

pub(crate) fn variadic_any_stable() -> Signature {
    Signature::new(TypeSignature::VariadicAny, Volatility::Stable)
}

pub(crate) fn any_one_volatile() -> Signature {
    Signature::new(TypeSignature::Any(1), Volatility::Volatile)
}

pub(crate) fn user_defined_volatile() -> Signature {
    Signature::user_defined(Volatility::Volatile)
}

pub(crate) fn stable_exact(arg_lists: &[&[ArrowDataType]]) -> Signature {
    Signature::one_of(
        arg_lists.iter().map(|args| TypeSignature::Exact(args.to_vec())).collect(),
        Volatility::Stable,
    )
}

pub(crate) fn arity(
    args: &ScalarFunctionArgs,
    accept: impl FnOnce(usize) -> bool,
    message: &str,
) -> DataFusionResult<()> {
    if accept(args.args.len()) {
        Ok(())
    } else {
        Err(DataFusionError::Plan(message.to_string()))
    }
}

pub(crate) fn utf8_scalar(value: &str) -> ColumnarValue {
    ColumnarValue::Scalar(datafusion::scalar::ScalarValue::Utf8(Some(value.to_owned())))
}

pub(crate) fn utf8_values(values: Vec<String>) -> ColumnarValue {
    ColumnarValue::Array(Arc::new(StringArray::from(values)))
}

pub(crate) fn utf8_rows(value: &str, rows: usize) -> ColumnarValue {
    let mut values = StringBuilder::with_capacity(rows, value.len());
    for _ in 0..rows {
        values.append_value(value);
    }
    ColumnarValue::Array(Arc::new(values.finish()))
}

pub(crate) fn utf8_nulls(rows: usize) -> ColumnarValue {
    let mut builder = StringBuilder::with_capacity(rows, 0);
    for _ in 0..rows {
        builder.append_null();
    }
    ColumnarValue::Array(Arc::new(builder.finish()))
}

pub(crate) fn bool_rows(value: bool, rows: usize) -> ColumnarValue {
    let mut builder = BooleanBuilder::with_capacity(rows);
    for _ in 0..rows {
        builder.append_value(value);
    }
    ColumnarValue::Array(Arc::new(builder.finish()))
}

pub(crate) fn int32_scalar(value: i32) -> ColumnarValue {
    ColumnarValue::Scalar(datafusion::scalar::ScalarValue::Int32(Some(value)))
}

/// Borrow text directly from scalar/array inputs; no intermediate per-row strings.
pub(crate) fn text_at(
    value: &ColumnarValue,
    row: usize,
) -> datafusion::error::Result<Option<&str>> {
    use datafusion::{
        arrow::array::{Array, LargeStringArray, StringViewArray},
        scalar::ScalarValue,
    };
    match value {
        ColumnarValue::Scalar(
            ScalarValue::Utf8(v) | ScalarValue::LargeUtf8(v) | ScalarValue::Utf8View(v),
        ) => Ok(v.as_deref()),
        ColumnarValue::Scalar(ScalarValue::Null) => Ok(None),
        ColumnarValue::Array(a) if a.is_null(row) => Ok(None),
        ColumnarValue::Array(a) => {
            if let Some(a) = a.as_any().downcast_ref::<StringArray>() {
                return Ok(Some(a.value(row)));
            }
            if let Some(a) = a.as_any().downcast_ref::<LargeStringArray>() {
                return Ok(Some(a.value(row)));
            }
            if let Some(a) = a.as_any().downcast_ref::<StringViewArray>() {
                return Ok(Some(a.value(row)));
            }
            Err(DataFusionError::Execution("expected text".into()))
        },
        _ => Err(DataFusionError::Execution("expected text".into())),
    }
}

pub(crate) fn bool_at(value: &ColumnarValue, row: usize) -> datafusion::error::Result<bool> {
    use datafusion::{
        arrow::array::{Array, BooleanArray},
        scalar::ScalarValue,
    };
    match value {
        ColumnarValue::Scalar(ScalarValue::Boolean(v)) => Ok(v.unwrap_or(false)),
        ColumnarValue::Scalar(ScalarValue::Null) => Ok(false),
        ColumnarValue::Array(a) => {
            let a = a
                .as_any()
                .downcast_ref::<BooleanArray>()
                .ok_or_else(|| DataFusionError::Execution("expected boolean".into()))?;
            Ok(!a.is_null(row) && a.value(row))
        },
        _ => Err(DataFusionError::Execution("expected boolean".into())),
    }
}

/// Preserve scalar inputs as scalars so DataFusion broadcasts only when needed.
pub(crate) fn map_text(
    args: &ScalarFunctionArgs,
    mut f: impl FnMut(usize) -> DataFusionResult<Option<String>>,
) -> DataFusionResult<ColumnarValue> {
    if args.args.iter().all(|arg| matches!(arg, ColumnarValue::Scalar(_))) {
        return Ok(ColumnarValue::Scalar(datafusion::scalar::ScalarValue::Utf8(f(0)?)));
    }
    let mut out = StringBuilder::new();
    for row in 0..args.number_rows {
        out.append_option(f(row)?);
    }
    Ok(ColumnarValue::Array(Arc::new(out.finish())))
}

macro_rules! scalar_udf_return {
    (utf8) => {
        $crate::sql::functions::scalar::utf8_type()
    };
    (int32) => {
        $crate::sql::functions::scalar::int32_type()
    };
}

macro_rules! scalar_udf_impl {
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident;
        name = $fn_name:expr;
        aliases = [$($alias:expr),* $(,)?];
        signature = $signature:expr;
        returns $ret:tt;
        bind = ();
        invoke($args:ident) $body:block
    ) => {
        $crate::sql::functions::scalar::scalar_udf_impl! {
            @finish
            $(#[$meta])*
            $vis struct $name;
            name = $fn_name;
            aliases = [$($alias),*];
            signature = $signature;
            returns $ret;
            invoke($args) $body
        }
    };

    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident;
        name = $fn_name:expr;
        aliases = [$($alias:expr),* $(,)?];
        signature = $signature:expr;
        returns $ret:tt;
        bind = ($this:ident);
        invoke($args:ident) $body:block
    ) => {
        $crate::sql::functions::scalar::scalar_udf_impl! {
            @finish
            $(#[$meta])*
            $vis struct $name;
            name = $fn_name;
            aliases = [$($alias),*];
            signature = $signature;
            returns $ret;
            invoke_self($this, $args) $body
        }
    };

    (
        @finish
        $(#[$meta:meta])*
        $vis:vis struct $name:ident;
        name = $fn_name:expr;
        aliases = [$($alias:expr),* $(,)?];
        signature = $signature:expr;
        returns $ret:tt;
        invoke($args:ident) $body:block
    ) => {
        $(#[$meta])*
        #[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
        $vis struct $name;

        impl $name {
            pub fn new() -> Self {
                Self
            }
        }

        impl datafusion::logical_expr::ScalarUDFImpl for $name {
            fn name(&self) -> &str {
                $fn_name
            }

            fn aliases(&self) -> &[::std::string::String] {
                static ALIASES: ::std::sync::OnceLock<::std::vec::Vec<::std::string::String>> =
                    ::std::sync::OnceLock::new();
                ALIASES.get_or_init(|| ::std::vec![$($alias.to_string()),*])
            }

            fn signature(&self) -> &datafusion::logical_expr::Signature {
                static SIGNATURE: ::std::sync::OnceLock<datafusion::logical_expr::Signature> =
                    ::std::sync::OnceLock::new();
                SIGNATURE.get_or_init(|| $signature)
            }

            fn return_type(
                &self,
                _args: &[kalamdb_commons::arrow_utils::ArrowDataType],
            ) -> datafusion::error::Result<kalamdb_commons::arrow_utils::ArrowDataType> {
                Ok($crate::sql::functions::scalar::scalar_udf_return!($ret))
            }

            fn invoke_with_args(
                &self,
                $args: datafusion::logical_expr::ScalarFunctionArgs,
            ) -> datafusion::error::Result<datafusion::logical_expr::ColumnarValue> {
                $body
            }
        }
    };

    (
        @finish
        $(#[$meta:meta])*
        $vis:vis struct $name:ident;
        name = $fn_name:expr;
        aliases = [$($alias:expr),* $(,)?];
        signature = $signature:expr;
        returns $ret:tt;
        invoke_self($this:ident, $args:ident) $body:block
    ) => {
        $(#[$meta])*
        #[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
        $vis struct $name;

        impl $name {
            pub fn new() -> Self {
                Self
            }
        }

        impl datafusion::logical_expr::ScalarUDFImpl for $name {
            fn name(&self) -> &str {
                $fn_name
            }

            fn aliases(&self) -> &[::std::string::String] {
                static ALIASES: ::std::sync::OnceLock<::std::vec::Vec<::std::string::String>> =
                    ::std::sync::OnceLock::new();
                ALIASES.get_or_init(|| ::std::vec![$($alias.to_string()),*])
            }

            fn signature(&self) -> &datafusion::logical_expr::Signature {
                static SIGNATURE: ::std::sync::OnceLock<datafusion::logical_expr::Signature> =
                    ::std::sync::OnceLock::new();
                SIGNATURE.get_or_init(|| $signature)
            }

            fn return_type(
                &self,
                _args: &[kalamdb_commons::arrow_utils::ArrowDataType],
            ) -> datafusion::error::Result<kalamdb_commons::arrow_utils::ArrowDataType> {
                Ok($crate::sql::functions::scalar::scalar_udf_return!($ret))
            }

            fn invoke_with_args(
                &self,
                $args: datafusion::logical_expr::ScalarFunctionArgs,
            ) -> datafusion::error::Result<datafusion::logical_expr::ColumnarValue> {
                let $this = self;
                $body
            }
        }
    };
}

macro_rules! scalar_udf {
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident;
        name = $fn_name:expr;
        aliases = [$($alias:expr),* $(,)?];
        signature = $signature:expr;
        returns $ret:tt;
        invoke($args:ident) $body:block
    ) => {
        $crate::sql::functions::scalar::scalar_udf_impl! {
            $(#[$meta])*
            $vis struct $name;
            name = $fn_name;
            aliases = [$($alias),*];
            signature = $signature;
            returns $ret;
            bind = ();
            invoke($args) $body
        }
    };

    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident;
        name = $fn_name:expr;
        aliases = [$($alias:expr),* $(,)?];
        signature = $signature:expr;
        returns $ret:tt;
        invoke(self = $this:ident, $args:ident) $body:block
    ) => {
        $crate::sql::functions::scalar::scalar_udf_impl! {
            $(#[$meta])*
            $vis struct $name;
            name = $fn_name;
            aliases = [$($alias),*];
            signature = $signature;
            returns $ret;
            bind = ($this);
            invoke($args) $body
        }
    };

    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident;
        name = $fn_name:expr;
        signature = $signature:expr;
        returns $ret:tt;
        invoke($args:ident) $body:block
    ) => {
        $crate::sql::functions::scalar::scalar_udf_impl! {
            $(#[$meta])*
            $vis struct $name;
            name = $fn_name;
            aliases = [];
            signature = $signature;
            returns $ret;
            bind = ();
            invoke($args) $body
        }
    };

    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident;
        name = $fn_name:expr;
        signature = $signature:expr;
        returns $ret:tt;
        invoke(self = $this:ident, $args:ident) $body:block
    ) => {
        $crate::sql::functions::scalar::scalar_udf_impl! {
            $(#[$meta])*
            $vis struct $name;
            name = $fn_name;
            aliases = [];
            signature = $signature;
            returns $ret;
            bind = ($this);
            invoke($args) $body
        }
    };
}

macro_rules! cached_signature {
    ($signature:expr) => {{
        static SIGNATURE: ::std::sync::OnceLock<datafusion::logical_expr::Signature> =
            ::std::sync::OnceLock::new();
        SIGNATURE.get_or_init(|| $signature)
    }};
}

macro_rules! cached_aliases {
    ($($alias:expr),* $(,)?) => {{
        static ALIASES: ::std::sync::OnceLock<::std::vec::Vec<::std::string::String>> =
            ::std::sync::OnceLock::new();
        ALIASES.get_or_init(|| ::std::vec![$($alias.to_string()),*])
    }};
}

pub(crate) use cached_aliases;
pub(crate) use cached_signature;
pub(crate) use scalar_udf;
pub(crate) use scalar_udf_impl;
pub(crate) use scalar_udf_return;
