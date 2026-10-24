//! Schema-known nested storage types for ordinal row encoding.

use datafusion_common::ScalarValue;

/// One field in a storage schema. Ordinal is the vec index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageField {
    pub name:            String,
    pub data_type:       StorageDataType,
    /// Dropped physical slot. Encoded as NULL; decoded values are discarded.
    pub dropped:         bool,
    /// Materialized missing-column default for rows written before `ADD COLUMN ... DEFAULT
    /// <expr>`.
    pub missing_default: Option<ScalarValue>,
}

impl StorageField {
    pub fn new(name: impl Into<String>, data_type: StorageDataType) -> Self {
        Self {
            name: name.into(),
            data_type,
            dropped: false,
            missing_default: None,
        }
    }

    pub fn with_missing_default(mut self, default: Option<ScalarValue>) -> Self {
        self.missing_default = default;
        self
    }

    /// Placeholder for a stable column_id hole (dropped or system column).
    pub fn dropped_slot() -> Self {
        Self {
            name:            String::new(),
            data_type:       StorageDataType::Boolean,
            dropped:         true,
            missing_default: None,
        }
    }
}

/// Physical nested type used by the row codec. Named `CREATE TYPE` identity lives in catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageDataType {
    Boolean,
    Int8,
    Int16,
    Int32,
    Int64,
    UInt8,
    UInt16,
    UInt32,
    UInt64,
    Float32,
    Float64,
    Utf8,
    Binary,
    Date32,
    Time64Microsecond,
    TimestampMillisecond,
    TimestampMicrosecond,
    TimestampNanosecond,
    Decimal {
        precision: u8,
        scale:     i8,
    },
    Embedding {
        dimension: i32,
    },
    /// Nested struct. Field slots are stable ordinals.
    Struct(Vec<StorageField>),
    /// One-dimensional list of the inner type.
    List(Box<StorageDataType>),
}

impl StorageDataType {
    /// Fixed byte width in the v2 row body fixed region, or `None` for variable-width / nested
    /// types.
    #[inline]
    pub const fn fixed_byte_width(&self) -> Option<usize> {
        match self {
            Self::Boolean | Self::Int8 | Self::UInt8 => Some(1),
            Self::Int16 | Self::UInt16 => Some(2),
            Self::Int32 | Self::UInt32 | Self::Float32 | Self::Date32 => Some(4),
            Self::Int64
            | Self::UInt64
            | Self::Float64
            | Self::Time64Microsecond
            | Self::TimestampMillisecond
            | Self::TimestampMicrosecond
            | Self::TimestampNanosecond => Some(8),
            Self::Decimal { .. } => Some(16),
            Self::Utf8
            | Self::Binary
            | Self::Embedding { .. }
            | Self::Struct(_)
            | Self::List(_) => None,
        }
    }
}

/// Precomputed per-version physical layout for the v2 row codec (`RowView`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageVersionLayout {
    /// Physical ordinal span (`fields.len()`).
    pub slot_span:                 usize,
    /// Fixed-width ordinals ordered widest-first for compact packing.
    pub fixed_permutation:         Vec<usize>,
    /// For each physical ordinal `0..slot_span`: `Some((fixed_byte_offset, byte_width))` if
    /// fixed-width.
    pub fixed_offsets:             Vec<Option<(usize, usize)>>,
    /// Total byte length of the packed fixed region.
    pub fixed_region_len:          usize,
    /// Physical ordinals of variable-width non-dropped columns in ordinal order.
    pub variable_ordinals:         Vec<usize>,
    /// For each physical ordinal `0..slot_span`: `Some(var_slot_index)` if variable-width.
    pub variable_slot_for_ordinal: Vec<Option<usize>>,
}

impl StorageVersionLayout {
    pub fn compute(fields: &[StorageField]) -> Self {
        let slot_span = fields.len();
        let mut fixed_candidates: Vec<(usize, usize)> = Vec::new();
        let mut variable_ordinals: Vec<usize> = Vec::new();
        let mut variable_slot_for_ordinal = vec![None; slot_span];

        for (ordinal, field) in fields.iter().enumerate() {
            if field.dropped {
                continue;
            }
            if let Some(width) = field.data_type.fixed_byte_width() {
                fixed_candidates.push((ordinal, width));
            } else {
                let var_idx = variable_ordinals.len();
                variable_ordinals.push(ordinal);
                variable_slot_for_ordinal[ordinal] = Some(var_idx);
            }
        }

        // Order fixed columns widest-first (stable by ordinal for ties).
        fixed_candidates.sort_by(|(ord_a, width_a), (ord_b, width_b)| {
            width_b.cmp(width_a).then_with(|| ord_a.cmp(ord_b))
        });

        let mut fixed_permutation = Vec::with_capacity(fixed_candidates.len());
        let mut fixed_offsets = vec![None; slot_span];
        let mut offset = 0usize;
        for (ordinal, width) in fixed_candidates {
            fixed_permutation.push(ordinal);
            fixed_offsets[ordinal] = Some((offset, width));
            offset += width;
        }

        Self {
            slot_span,
            fixed_permutation,
            fixed_offsets,
            fixed_region_len: offset,
            variable_ordinals,
            variable_slot_for_ordinal,
        }
    }
}

/// Schema used to encode/decode a table row by ordinal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageSchema {
    pub version:            u16,
    pub fields:             Vec<StorageField>,
    pub layout:             StorageVersionLayout,
    pub historical_layouts: std::collections::BTreeMap<u16, StorageVersionLayout>,
}

impl StorageSchema {
    pub fn new(version: u16, fields: Vec<StorageField>) -> Self {
        let layout = StorageVersionLayout::compute(&fields);
        let mut historical_layouts = std::collections::BTreeMap::new();
        historical_layouts.insert(version, layout.clone());
        Self {
            version,
            fields,
            layout,
            historical_layouts,
        }
    }

    pub fn register_historical_version(&mut self, version: u16, fields: &[StorageField]) {
        self.historical_layouts.insert(version, StorageVersionLayout::compute(fields));
    }

    pub fn layout_for_version(&self, version: u16) -> Option<&StorageVersionLayout> {
        if version == self.version {
            Some(&self.layout)
        } else {
            self.historical_layouts.get(&version)
        }
    }

    pub fn field(&self, index: usize) -> Option<&StorageField> {
        self.fields.get(index)
    }

    pub fn layout(&self) -> &StorageVersionLayout {
        &self.layout
    }
}
