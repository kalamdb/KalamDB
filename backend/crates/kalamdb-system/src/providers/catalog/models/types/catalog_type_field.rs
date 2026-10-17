use kalamdb_commons::{
    datatypes::KalamDataType,
    models::{TypeFieldId, TypeId},
};
use kalamdb_macros::table;
use serde::{Deserialize, Serialize};

/// Persisted `system.type_fields` row (composite field, implicit row field, or enum label).
#[table(
    name = "type_fields",
    comment = "Fields and enum labels for catalog types"
)]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CatalogTypeField {
    #[column(
        id = 1,
        ordinal = 1,
        data_type(KalamDataType::Text),
        nullable = false,
        primary_key = true,
        default = "None",
        comment = "TypeId:slot for composites; TypeId:name for enum labels"
    )]
    pub type_field_id:    TypeFieldId,
    #[column(
        id = 2,
        ordinal = 2,
        data_type(KalamDataType::Text),
        nullable = false,
        primary_key = false,
        default = "None",
        comment = "Parent catalog type"
    )]
    pub type_id:          TypeId,
    #[column(
        id = 3,
        ordinal = 3,
        data_type(KalamDataType::Text),
        nullable = false,
        primary_key = false,
        default = "None",
        comment = "Field or enum label name"
    )]
    pub name:             String,
    #[column(
        id = 4,
        ordinal = 4,
        data_type(KalamDataType::Int),
        nullable = false,
        primary_key = false,
        default = "None",
        comment = "Declaration order"
    )]
    pub ordinal:          i32,
    #[column(
        id = 5,
        ordinal = 5,
        data_type(KalamDataType::Text),
        nullable = true,
        primary_key = false,
        default = "None",
        comment = "Named type reference when the field is not a primitive"
    )]
    #[serde(default)]
    pub field_type_id:    Option<TypeId>,
    #[column(
        id = 6,
        ordinal = 6,
        data_type(KalamDataType::Text),
        nullable = false,
        primary_key = false,
        default = "None",
        comment = "Resolved type name (primitive or named)"
    )]
    pub type_name:        String,
    #[column(
        id = 7,
        ordinal = 7,
        data_type(KalamDataType::Boolean),
        nullable = false,
        primary_key = false,
        default = "None",
        comment = "True when the field is an array"
    )]
    pub is_array:         bool,
    #[column(
        id = 8,
        ordinal = 8,
        data_type(KalamDataType::Boolean),
        nullable = false,
        primary_key = false,
        default = "None",
        comment = "NOT NULL"
    )]
    pub not_null:         bool,
    #[column(
        id = 9,
        ordinal = 9,
        data_type(KalamDataType::Boolean),
        nullable = false,
        primary_key = false,
        default = "None",
        comment = "NONEMPTY for arrays"
    )]
    pub nonempty:         bool,
    #[column(
        id = 10,
        ordinal = 10,
        data_type(KalamDataType::Json),
        nullable = true,
        primary_key = false,
        default = "None",
        comment = "Builtin KalamDataType when the field is not a named CREATE TYPE"
    )]
    #[serde(default)]
    pub data_type:        Option<KalamDataType>,
    #[column(
        id = 11,
        ordinal = 11,
        data_type(KalamDataType::Int),
        nullable = false,
        primary_key = false,
        default = "None",
        comment = "Stable physical slot; never reused"
    )]
    #[serde(default)]
    pub slot:             i32,
    #[column(
        id = 12,
        ordinal = 12,
        data_type(KalamDataType::Boolean),
        nullable = false,
        primary_key = false,
        default = "None",
        comment = "Tombstone; slot is not reused"
    )]
    #[serde(default)]
    pub dropped:          bool,
    #[column(
        id = 13,
        ordinal = 13,
        data_type(KalamDataType::Boolean),
        nullable = false,
        primary_key = false,
        default = "None",
        comment = "List element nullability"
    )]
    #[serde(default = "kalamdb_commons::LogicalTypeRef::default_element_nullable")]
    pub element_nullable: bool,
}

impl kalamdb_commons::KSerializable for CatalogTypeField {}

impl CatalogTypeField {
    pub fn new(
        type_id: TypeId,
        name: impl Into<String>,
        ordinal: i32,
        field_type_id: Option<TypeId>,
        data_type: Option<KalamDataType>,
        type_name: impl Into<String>,
        is_array: bool,
        not_null: bool,
        nonempty: bool,
    ) -> Result<Self, String> {
        let name = name.into();
        let slot = if ordinal > 0 { ordinal } else { 1 };
        let type_field_id =
            TypeFieldId::from_slot(&type_id, slot).or_else(|_| TypeFieldId::new(&type_id, &name))?;
        Ok(Self {
            type_field_id,
            type_id,
            name,
            ordinal,
            field_type_id,
            type_name: type_name.into(),
            is_array,
            not_null,
            nonempty,
            data_type,
            slot,
            dropped: false,
            element_nullable: !not_null || is_array,
        })
    }

    /// Construct an enum member with a stable physical slot (`type_field_id = {type_id}:{slot}`)
    /// decoupled from its mutable display label (`name`) and sort position (`ordinal`).
    pub fn enum_value_with_slot(
        type_id: TypeId,
        name: impl Into<String>,
        slot: i32,
        ordinal: i32,
    ) -> Result<Self, String> {
        let name = name.into();
        let slot = if slot > 0 { slot } else { 1 };
        let type_field_id =
            TypeFieldId::from_slot(&type_id, slot).or_else(|_| TypeFieldId::new(&type_id, &name))?;
        Ok(Self {
            type_field_id,
            type_id,
            name: name.clone(),
            ordinal,
            field_type_id: None,
            type_name: name,
            is_array: false,
            not_null: true,
            nonempty: false,
            data_type: None,
            slot,
            dropped: false,
            element_nullable: false,
        })
    }

    pub fn physical_slot(&self) -> i32 {
        if self.slot > 0 {
            self.slot
        } else if self.ordinal > 0 {
            self.ordinal
        } else {
            1
        }
    }

    pub fn type_ref(&self) -> Result<kalamdb_commons::LogicalTypeRef, String> {
        if self.dropped {
            return Err(format!("type field '{}' is dropped", self.name));
        }
        if self.field_type_id.is_none() {
            let bare_name = self.type_name.trim_end_matches("[]").trim();
            if bare_name.eq_ignore_ascii_case("jsonb") {
                let inner = kalamdb_commons::LogicalTypeRef::jsonb();
                return Ok(if self.is_array {
                    kalamdb_commons::LogicalTypeRef::array(inner, self.element_nullable)
                } else {
                    inner
                });
            }
            if bare_name.eq_ignore_ascii_case("timestamptz")
                || bare_name.eq_ignore_ascii_case("timestamp with time zone")
                || self.builtin_data_type() == Some(KalamDataType::DateTime)
            {
                let inner = kalamdb_commons::LogicalTypeRef::timestamptz();
                return Ok(if self.is_array {
                    kalamdb_commons::LogicalTypeRef::array(inner, self.element_nullable)
                } else {
                    inner
                });
            }
        }
        kalamdb_commons::LogicalTypeRef::stored(
            &format!("type field '{}'", self.name),
            self.field_type_id.as_ref(),
            self.builtin_data_type(),
            self.is_array,
            self.element_nullable,
        )
    }

    pub fn from_column(
        type_id: &TypeId,
        column: &kalamdb_commons::schemas::ColumnDefinition,
    ) -> Result<Self, String> {
        let ordinal = i32::try_from(column.ordinal_position).unwrap_or(i32::MAX);
        let mut field = Self::new(
            type_id.clone(),
            column.column_name.clone(),
            ordinal,
            column.named_type_id().cloned(),
            if column.named_type_id().is_some() {
                None
            } else {
                Some(column.data_type())
            },
            column.logical_type.peel_list().sql_name(),
            column.is_array(),
            !column.is_nullable,
            false,
        )?;
        field.element_nullable = column.element_nullable();
        Ok(field)
    }

    pub fn builtin_data_type(&self) -> Option<KalamDataType> {
        self.data_type.or_else(|| {
            if self.field_type_id.is_some() {
                None
            } else {
                KalamDataType::from_sql_name(&self.type_name)
            }
        })
    }
}
