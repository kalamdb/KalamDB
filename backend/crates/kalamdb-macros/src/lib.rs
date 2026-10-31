use proc_macro::TokenStream;
use quote::quote;
use syn::{
    parse::{Parse, ParseStream},
    parse_macro_input,
    spanned::Spanned,
    Attribute, Ident, ItemStruct, LitBool, LitInt, LitStr, Result,
};

/// Generate `definition()`, `arrow_schema()`, and primary-key helpers from column metadata.
///
/// Persisted tables keep explicit ids and ordinals because field order can differ from
/// storage order:
///
/// ```ignore
/// #[column(id = 10, ordinal = 12, data_type(KalamDataType::Timestamp), comment = "created")]
/// pub created_at: i64,
/// ```
///
/// `nullable`, `primary_key`, and `default` can be omitted. Nullability then follows
/// `Option`, the primary key defaults to false, and the default value is none.
/// Omit `data_type` to infer it from the field type (`String` is text, `i64` is bigint,
/// `Timestamp` is timestamp, `Value` or `Json` is json, and names ending in `Id` are text).
///
/// Virtual views use `view = true` (`infer = true` is the same flag). Field order is
/// the column id and ordinal unless `ordinal` is set. `Option` is nullable. The SQL
/// type comes from the field type. A bare type overrides it: `#[column(Timestamp)]`.
/// A doc comment, when present, is the column comment. No `#[column]` is required.
/// Built-in schemas are `namespace = SchemaId::system()`, `SchemaId::pg_catalog()`,
/// or `SchemaId::information_schema()`. View structs also implement
/// [`kalamdb_commons::schemas::CatalogRow`].
#[proc_macro_attribute]
pub fn table(attr: TokenStream, item: TokenStream) -> TokenStream {
    let table_args = parse_macro_input!(attr as TableArgs);
    let mut item_struct = parse_macro_input!(item as ItemStruct);

    let struct_ident = &item_struct.ident;

    let mut columns = Vec::new();
    let mut column_ids = std::collections::HashSet::new();
    let mut ordinals = std::collections::HashSet::new();
    for (field_index, field) in item_struct.fields.iter_mut().enumerate() {
        let column_attr = extract_column_attr(&mut field.attrs);
        let mut column_attr = match column_attr {
            Some(attr) => attr,
            None if table_args.view => ColumnArgs::default(),
            None => {
                return syn::Error::new_spanned(
                    &field.ident,
                    "missing #[column(...)] attribute for field",
                )
                .to_compile_error()
                .into();
            },
        };

        if let Some(err) = column_attr.parse_error.take() {
            return err.to_compile_error().into();
        }

        if let Err(err) = column_attr.complete(
            field_index,
            table_args.view,
            &field.ty,
            &field.attrs,
            field.ident.as_ref(),
        ) {
            return err.to_compile_error().into();
        }

        let field_ident = match &field.ident {
            Some(ident) => ident,
            None => {
                return syn::Error::new_spanned(
                    &field.ty,
                    "tuple structs are not supported by #[table]",
                )
                .to_compile_error()
                .into();
            },
        };

        if !column_ids.insert(column_attr.column_id) {
            return syn::Error::new_spanned(
                &field.ident,
                format!("duplicate column id {}", column_attr.column_id),
            )
            .to_compile_error()
            .into();
        }

        if !ordinals.insert(column_attr.ordinal_position) {
            return syn::Error::new_spanned(
                &field.ident,
                format!("duplicate column ordinal {}", column_attr.ordinal_position),
            )
            .to_compile_error()
            .into();
        }

        columns.push(ColumnSpec {
            field_name: field_ident.clone(),
            args: column_attr,
        });
    }

    columns.sort_by_key(|spec| spec.args.ordinal_position);

    let column_defs = columns.iter().map(|spec| {
        let column_id = spec.args.column_id;
        let column_name = spec.field_name.to_string();
        let ordinal_position = spec.args.ordinal_position;
        let data_type = spec.args.data_type_expr();
        let is_nullable = spec.args.is_nullable;
        let is_primary_key = spec.args.is_primary_key;
        let is_partition_key = spec.args.is_partition_key;
        let default_value = spec.args.default_expr();
        let column_comment = spec.args.comment_expr();

        quote! {
            kalamdb_commons::schemas::ColumnDefinition::new(
                #column_id,
                #column_name,
                #ordinal_position,
                #data_type,
                #is_nullable,
                #is_primary_key,
                #is_partition_key,
                #default_value,
                #column_comment)
        }
    });

    let TableArgs {
        name,
        namespace,
        table_type,
        access_level,
        comment,
        view,
    } = table_args;

    if access_level.is_some() {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "#[table(access_level = ...)] is not supported. Shared tables use FORCE row-level \
             security; grant access with CREATE POLICY",
        )
        .to_compile_error()
        .into();
    }

    let namespace = namespace.unwrap_or(NamespaceArg::System);
    let namespace_expr = namespace.expr();
    let table_name = name.clone();
    let namespace_for_error = namespace.display_name();
    let table_type_expr = table_type.expr();
    let table_comment = match comment {
        Some(comment) => quote!(Some(#comment.to_string())),
        None => quote!(None),
    };
    let ordinal_names: Vec<String> =
        columns.iter().map(|spec| spec.field_name.to_string()).collect();
    let primary_key_names: Vec<String> = columns
        .iter()
        .filter(|spec| spec.args.is_primary_key)
        .map(|spec| spec.field_name.to_string())
        .collect();
    let error_message =
        format!("Failed to create {}.{} table definition", namespace_for_error, table_name);
    let schema_error =
        format!("Failed to create {}.{} Arrow schema", namespace_for_error, table_name);
    let arrow_schema_fn = arrow_schema_fn(&columns, &schema_error);
    let catalog_row_impl = if view {
        catalog_row_impl(struct_ident, &columns)
    } else {
        quote!()
    };

    let output = quote! {
        #item_struct

        impl #struct_ident {
            pub fn definition() -> kalamdb_commons::schemas::TableDefinition {
                let columns = vec![
                    #(#column_defs,)*
                ];

                let mut table_def = kalamdb_commons::schemas::TableDefinition::new_with_defaults(
                    #namespace_expr,
                    kalamdb_commons::TableName::new(#table_name),
                    #table_type_expr,
                    columns,
                    #table_comment)
                .expect(#error_message);

                table_def
            }

            #arrow_schema_fn

            /// Primary-key column when the table has exactly one.
            pub fn primary_key_column() -> Option<&'static str> {
                match Self::primary_key_columns() {
                    [column] => Some(*column),
                    _ => None,
                }
            }

            /// Primary-key column names in ordinal order.
            pub fn primary_key_columns() -> &'static [&'static str] {
                &[#(#primary_key_names),*]
            }

            /// Column names in `ordinal` order.
            pub fn ordinal_column_order() -> &'static [&'static str] {
                &[#(#ordinal_names),*]
            }
        }

        #catalog_row_impl
    };

    output.into()
}

struct ColumnSpec {
    field_name: Ident,
    args: ColumnArgs,
}

fn extract_column_attr(attrs: &mut Vec<Attribute>) -> Option<ColumnArgs> {
    let mut column_args = None;
    let mut new_attrs = Vec::with_capacity(attrs.len());

    for attr in attrs.drain(..) {
        if attr.path().is_ident("column") {
            match attr.parse_args::<ColumnArgs>() {
                Ok(parsed) => column_args = Some(parsed),
                Err(err) => {
                    column_args = Some(ColumnArgs::error(err));
                },
            }
        } else {
            new_attrs.push(attr);
        }
    }

    *attrs = new_attrs;
    column_args
}

#[derive(Default)]
struct TableArgs {
    name: String,
    namespace: Option<NamespaceArg>,
    table_type: TableKind,
    access_level: Option<TableAccessKind>,
    comment: Option<String>,
    /// Virtual view. Column id and ordinal default to field position. The struct
    /// implements `CatalogRow`.
    view: bool,
}

impl Parse for TableArgs {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let mut args = TableArgs::default();

        while !input.is_empty() {
            let key: syn::Path = input.parse()?;
            input.parse::<syn::Token![=]>()?;

            let key_ident = key
                .get_ident()
                .ok_or_else(|| syn::Error::new_spanned(&key, "unsupported #[table] argument"))?
                .to_string();

            match key_ident.as_str() {
                "name" => {
                    let value: LitStr = input.parse()?;
                    args.name = value.value();
                },
                "namespace" => {
                    args.namespace = Some(parse_namespace(input)?);
                },
                "table_type" => {
                    args.table_type = parse_table_kind_value(input)?;
                },
                "access_level" => {
                    args.access_level = Some(parse_table_access_value(input)?);
                },
                "comment" => {
                    let value: LitStr = input.parse()?;
                    args.comment = Some(value.value());
                },
                "view" | "infer" => {
                    let value: LitBool = input.parse()?;
                    args.view = value.value();
                },
                _ => {
                    return Err(syn::Error::new_spanned(&key, "unsupported #[table] argument"));
                },
            }

            if input.peek(syn::Token![,]) {
                input.parse::<syn::Token![,]>()?;
            }
        }

        if args.name.is_empty() {
            return Err(syn::Error::new(input.span(), "#[table] requires name = \"...\""));
        }

        Ok(args)
    }
}

enum NamespaceArg {
    System,
    PgCatalog,
    InformationSchema,
    Other(String),
}

impl NamespaceArg {
    fn display_name(&self) -> String {
        match self {
            Self::System => "system".to_string(),
            Self::PgCatalog => "pg_catalog".to_string(),
            Self::InformationSchema => "information_schema".to_string(),
            Self::Other(name) => name.clone(),
        }
    }

    fn expr(&self) -> proc_macro2::TokenStream {
        match self {
            Self::System => quote!(kalamdb_commons::SchemaId::system()),
            Self::PgCatalog => quote!(kalamdb_commons::SchemaId::pg_catalog()),
            Self::InformationSchema => quote!(kalamdb_commons::SchemaId::information_schema()),
            Self::Other(name) => quote!(kalamdb_commons::SchemaId::new(#name)),
        }
    }
}

fn parse_namespace(input: ParseStream<'_>) -> Result<NamespaceArg> {
    if input.peek(LitStr) {
        let value: LitStr = input.parse()?;
        let name = value.value();
        return match name.to_ascii_lowercase().as_str() {
            "system" | "pg_catalog" | "information_schema" => Err(syn::Error::new_spanned(
                &value,
                format!("use SchemaId::{name}() instead of a string namespace"),
            )),
            _ => Ok(NamespaceArg::Other(name)),
        };
    }

    let expr: syn::Expr = input.parse()?;
    let path = match &expr {
        syn::Expr::Path(path) => &path.path,
        syn::Expr::Call(call) => match call.func.as_ref() {
            syn::Expr::Path(path) if call.args.is_empty() => &path.path,
            _ => {
                return Err(syn::Error::new_spanned(
                    &expr,
                    "namespace must be SchemaId::system(), SchemaId::pg_catalog(), SchemaId::information_schema(), or a string",
                ));
            }
        },
        _ => {
            return Err(syn::Error::new_spanned(
                &expr,
                "namespace must be SchemaId::system(), SchemaId::pg_catalog(), SchemaId::information_schema(), or a string",
            ));
        }
    };
    let ident = path
        .segments
        .last()
        .ok_or_else(|| syn::Error::new_spanned(path, "expected SchemaId constructor"))?
        .ident
        .to_string();
    match ident.as_str() {
        "system" => Ok(NamespaceArg::System),
        "pg_catalog" => Ok(NamespaceArg::PgCatalog),
        "information_schema" => Ok(NamespaceArg::InformationSchema),
        _ => Err(syn::Error::new_spanned(
            path,
            "expected SchemaId::system(), SchemaId::pg_catalog(), or SchemaId::information_schema()",
        )),
    }
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum TableKind {
    User,
    Shared,
    Stream,
    #[default]
    System,
}

impl TableKind {
    fn parse(value: &LitStr) -> Result<Self> {
        match value.value().to_lowercase().as_str() {
            "user" => Ok(Self::User),
            "shared" => Ok(Self::Shared),
            "stream" => Ok(Self::Stream),
            "system" => Ok(Self::System),
            _ => Err(syn::Error::new_spanned(
                value,
                "unsupported table_type; expected user|shared|stream|system",
            )),
        }
    }

    fn expr(&self) -> proc_macro2::TokenStream {
        match self {
            Self::User => quote!(kalamdb_commons::schemas::TableType::User),
            Self::Shared => quote!(kalamdb_commons::schemas::TableType::Shared),
            Self::Stream => quote!(kalamdb_commons::schemas::TableType::Stream),
            Self::System => quote!(kalamdb_commons::schemas::TableType::System),
        }
    }
}

#[derive(Clone, Copy)]
enum TableAccessKind {
    Public,
    Private,
    Restricted,
    Dba,
}

impl TableAccessKind {
    fn parse_str(value: &str, span: proc_macro2::Span) -> Result<Self> {
        match value.to_lowercase().as_str() {
            "public" => Ok(Self::Public),
            "private" => Ok(Self::Private),
            "restricted" => Ok(Self::Restricted),
            "dba" => Ok(Self::Dba),
            _ => Err(syn::Error::new(
                span,
                "unsupported access_level; expected public|private|restricted|dba",
            )),
        }
    }

    fn parse(value: &LitStr) -> Result<Self> {
        Self::parse_str(&value.value(), value.span())
    }
}

fn parse_table_kind_value(input: ParseStream<'_>) -> Result<TableKind> {
    if input.peek(LitStr) {
        let value: LitStr = input.parse()?;
        return TableKind::parse(&value);
    }

    let value: syn::Path = input.parse()?;
    let ident = value
        .segments
        .last()
        .ok_or_else(|| syn::Error::new_spanned(&value, "expected table type enum variant"))?
        .ident
        .to_string();
    let lit = LitStr::new(&ident, value.span());
    TableKind::parse(&lit)
}

fn parse_table_access_value(input: ParseStream<'_>) -> Result<TableAccessKind> {
    if input.peek(LitStr) {
        let value: LitStr = input.parse()?;
        return TableAccessKind::parse(&value);
    }

    let value: syn::Path = input.parse()?;
    let ident = value
        .segments
        .last()
        .ok_or_else(|| syn::Error::new_spanned(&value, "expected access level enum variant"))?
        .ident
        .to_string();
    TableAccessKind::parse_str(&ident, value.span())
}

#[derive(Default)]
struct ColumnArgs {
    column_id: u64,
    ordinal_position: u32,
    data_type: Option<syn::Expr>,
    is_nullable: bool,
    is_primary_key: bool,
    is_partition_key: bool,
    default_value: String,
    comment: Option<String>,
    seen_nullable: bool,
    seen_primary_key: bool,
    type_override: Option<Ident>,
    value_kind: Option<ValueKind>,
    parse_error: Option<syn::Error>,
}

impl ColumnArgs {
    fn error(err: syn::Error) -> Self {
        ColumnArgs {
            parse_error: Some(err),
            ..Default::default()
        }
    }

    fn data_type_expr(&self) -> proc_macro2::TokenStream {
        if let Some(err) = &self.parse_error {
            return err.to_compile_error();
        }

        match &self.data_type {
            Some(expr) => quote!(#expr),
            None => syn::Error::new(
                proc_macro2::Span::call_site(),
                "#[column] requires data_type = KalamDataType::...",
            )
            .to_compile_error(),
        }
    }

    fn default_expr(&self) -> proc_macro2::TokenStream {
        if let Some(err) = &self.parse_error {
            return err.to_compile_error();
        }

        match self.default_value.as_str() {
            "None" => quote!(kalamdb_commons::schemas::ColumnDefault::None),
            "Literal(false)" => quote!(kalamdb_commons::schemas::ColumnDefault::Literal(
                serde_json::Value::Bool(false)
            )),
            "Literal(true)" => quote!(kalamdb_commons::schemas::ColumnDefault::Literal(
                serde_json::Value::Bool(true)
            )),
            other if other.starts_with("Function(") && other.ends_with(')') => {
                let inner = &other["Function(".len()..other.len() - 1];
                if inner.trim().is_empty() {
                    syn::Error::new(
                        proc_macro2::Span::call_site(),
                        "Function() default requires a function name",
                    )
                    .to_compile_error()
                } else {
                    quote!(kalamdb_commons::schemas::ColumnDefault::function(#inner, vec![]))
                }
            },
            other => syn::Error::new(
                proc_macro2::Span::call_site(),
                format!(
                    "unsupported default '{}'; expected None, Literal(true|false), or \
                     Function(NAME)",
                    other
                ),
            )
            .to_compile_error(),
        }
    }

    fn comment_expr(&self) -> proc_macro2::TokenStream {
        if let Some(err) = &self.parse_error {
            return err.to_compile_error();
        }

        match &self.comment {
            Some(comment) => quote!(Some(#comment.to_string())),
            None => quote!(None),
        }
    }
}

impl Parse for ColumnArgs {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let mut args = ColumnArgs::default();
        let mut seen = std::collections::HashSet::new();

        while !input.is_empty() {
            if input.peek(Ident) {
                let fork = input.fork();
                let peeked: Ident = fork.parse()?;
                let named = fork.peek(syn::Token![=])
                    || (peeked == "data_type" && fork.peek(syn::token::Paren));
                if !named {
                    let type_name: Ident = input.parse()?;
                    if args.type_override.is_some() {
                        return Err(syn::Error::new_spanned(
                            type_name,
                            "only one bare SQL type is allowed in #[column]",
                        ));
                    }
                    args.type_override = Some(type_name);
                    if input.peek(syn::Token![,]) {
                        input.parse::<syn::Token![,]>()?;
                    }
                    continue;
                }
            }

            let key: Ident = input.parse()?;

            let key_str = key.to_string();
            if !seen.insert(key_str.clone()) {
                return Err(syn::Error::new_spanned(key, "duplicate #[column] argument"));
            }

            if key_str == "data_type" && input.peek(syn::token::Paren) {
                let content;
                syn::parenthesized!(content in input);
                let value: syn::Expr = content.parse()?;
                args.data_type = Some(value);
            } else {
                input.parse::<syn::Token![=]>()?;

                match key_str.as_str() {
                    "id" => {
                        let value: LitInt = input.parse()?;
                        args.column_id = value.base10_parse()?;
                    },
                    "ordinal" => {
                        let value: LitInt = input.parse()?;
                        args.ordinal_position = value.base10_parse()?;
                    },
                    "data_type" => {
                        return Err(syn::Error::new_spanned(
                            key,
                            "data_type must use list syntax: data_type(KalamDataType::...)",
                        ));
                    },
                    "nullable" => {
                        let value: LitBool = input.parse()?;
                        args.is_nullable = value.value();
                        args.seen_nullable = true;
                    },
                    "primary_key" => {
                        let value: LitBool = input.parse()?;
                        args.is_primary_key = value.value();
                        args.seen_primary_key = true;
                    },
                    "default" => {
                        let value: LitStr = input.parse()?;
                        args.default_value = value.value();
                    },
                    "comment" => {
                        let value: LitStr = input.parse()?;
                        args.comment = Some(value.value());
                    },
                    _ => {
                        return Err(syn::Error::new_spanned(key, "unsupported #[column] argument"));
                    },
                }
            }

            if input.peek(syn::Token![,]) {
                input.parse::<syn::Token![,]>()?;
            }
        }

        Ok(args)
    }
}

impl ColumnArgs {
    fn complete(
        &mut self,
        field_index: usize,
        view: bool,
        ty: &syn::Type,
        attrs: &[Attribute],
        field_name: Option<&Ident>,
    ) -> Result<()> {
        let span = field_name.map(|name| name.span()).unwrap_or_else(proc_macro2::Span::call_site);
        let (type_is_option, inner_ty) = unwrap_option(ty);

        if self.column_id == 0 {
            if !view {
                return Err(syn::Error::new(
                    span,
                    "#[column] requires id = <u64> on persisted tables",
                ));
            }
            self.column_id = u64::try_from(field_index + 1).unwrap_or(u64::MAX);
        }
        if self.ordinal_position == 0 {
            if !view {
                return Err(syn::Error::new(
                    span,
                    "#[column] requires ordinal = <u32> on persisted tables",
                ));
            }
            self.ordinal_position = u32::try_from(field_index + 1).unwrap_or(u32::MAX);
        }
        if let Some(type_name) = self.type_override.clone() {
            if self.data_type.is_some() {
                return Err(syn::Error::new(
                    span,
                    "#[column] bare type and data_type(...) cannot both be set",
                ));
            }
            self.data_type =
                Some(sql_type_expr(&type_name).map_err(|message| syn::Error::new(span, message))?);
        }
        if !self.seen_nullable {
            self.is_nullable = type_is_option;
        }
        if !self.seen_primary_key {
            self.is_primary_key = false;
        }
        if self.default_value.is_empty() {
            self.default_value = "None".to_string();
        }
        if self.data_type.is_none() {
            self.data_type = Some(infer_sql_type(inner_ty).map_err(|message| {
                syn::Error::new(
                    span,
                    format!(
                        "{message}. Set #[column(data_type(KalamDataType::...))] or use a mapped \
                         field type"
                    ),
                )
            })?);
        }
        if self.comment.is_none() && view {
            self.comment = doc_comment(attrs);
        }
        if view {
            let sql_variant = self.data_type.as_ref().and_then(expr_last_ident).unwrap_or_default();
            let kind =
                classify_value(inner_ty).map_err(|message| syn::Error::new(span, message))?;
            self.value_kind = Some(
                align_value_kind(kind, &sql_variant)
                    .map_err(|message| syn::Error::new(span, message))?,
            );
        }
        Ok(())
    }
}

fn unwrap_option(ty: &syn::Type) -> (bool, &syn::Type) {
    let ty = peel_type(ty);
    let syn::Type::Path(path) = ty else {
        return (false, ty);
    };
    let Some(segment) = path.path.segments.last() else {
        return (false, ty);
    };
    if segment.ident != "Option" {
        return (false, ty);
    }
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
        return (false, ty);
    };
    let Some(syn::GenericArgument::Type(inner)) = args.args.first() else {
        return (false, ty);
    };
    (true, inner)
}

fn peel_type(ty: &syn::Type) -> &syn::Type {
    match ty {
        syn::Type::Group(group) => peel_type(&group.elem),
        syn::Type::Paren(paren) => peel_type(&paren.elem),
        other => other,
    }
}

fn infer_sql_type(ty: &syn::Type) -> std::result::Result<syn::Expr, String> {
    let ty = peel_type(ty);
    let syn::Type::Path(path) = ty else {
        return Err("cannot infer a SQL type from this field type".to_string());
    };
    let Some(segment) = path.path.segments.last() else {
        return Err("cannot infer a SQL type from this field type".to_string());
    };
    if segment.ident == "Vec" {
        return syn::parse_str("kalamdb_commons::datatypes::KalamDataType::Json")
            .map_err(|error| error.to_string());
    }
    if !matches!(segment.arguments, syn::PathArguments::None) {
        return Err(format!("cannot infer a SQL type for {}", segment.ident));
    }
    let variant = match segment.ident.to_string().as_str() {
        "String" => "Text",
        "bool" => "Boolean",
        "i64" | "u64" => "BigInt",
        "i32" | "u32" => "Int",
        "i16" | "u16" | "i8" | "u8" => "SmallInt",
        "f64" => "Double",
        "f32" => "Float",
        "Timestamp" => "Timestamp",
        "Value" | "Json" => "Json",
        other if other.ends_with("Id") || other == "TableName" || other == "NodeId" => "Text",
        other => {
            return Err(format!("cannot infer a SQL type for {other}"));
        },
    };
    syn::parse_str(&format!("kalamdb_commons::datatypes::KalamDataType::{variant}"))
        .map_err(|error| error.to_string())
}

fn doc_comment(attrs: &[Attribute]) -> Option<String> {
    let mut lines = Vec::new();
    for attr in attrs {
        if !attr.path().is_ident("doc") {
            continue;
        }
        let syn::Meta::NameValue(name_value) = &attr.meta else {
            continue;
        };
        let syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(value),
            ..
        }) = &name_value.value
        else {
            continue;
        };
        let line = value.value();
        let line = line.trim();
        if !line.is_empty() {
            lines.push(line.to_string());
        }
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines.join(" "))
    }
}

#[derive(Clone, Copy)]
enum ValueKind {
    Boolean,
    Int16,
    Int32,
    Int64,
    Float32,
    Float64,
    Utf8,
    Timestamp,
    Json,
    ListInt64,
    ListUtf8,
}

impl ValueKind {
    fn sql_name(self) -> &'static str {
        match self {
            Self::Boolean => "Boolean",
            Self::Int16 => "SmallInt",
            Self::Int32 => "Int",
            Self::Int64 => "BigInt",
            Self::Float32 => "Float",
            Self::Float64 => "Double",
            Self::Utf8 => "Text",
            Self::Timestamp => "Timestamp",
            Self::Json | Self::ListInt64 | Self::ListUtf8 => "Json",
        }
    }
}

fn sql_type_expr(name: &Ident) -> std::result::Result<syn::Expr, String> {
    let variant = match name.to_string().as_str() {
        "Boolean" | "Int" | "BigInt" | "Double" | "Float" | "Text" | "Timestamp" | "Date"
        | "Time" | "Json" | "Bytes" | "Uuid" | "SmallInt" | "File" => name.to_string(),
        "DateTime" => {
            return Err(
                "DateTime override is not supported on catalog views; use Timestamp".to_string()
            );
        },
        other => return Err(format!("unsupported SQL type {other}")),
    };
    syn::parse_str(&format!("kalamdb_commons::datatypes::KalamDataType::{variant}"))
        .map_err(|error| error.to_string())
}

fn expr_last_ident(expr: &syn::Expr) -> Option<String> {
    let syn::Expr::Path(path) = expr else {
        return None;
    };
    path.path.segments.last().map(|segment| segment.ident.to_string())
}

fn classify_value(ty: &syn::Type) -> std::result::Result<ValueKind, String> {
    let ty = peel_type(ty);
    let syn::Type::Path(path) = ty else {
        return Err("cannot build a catalog batch from this field type".to_string());
    };
    let Some(segment) = path.path.segments.last() else {
        return Err("cannot build a catalog batch from this field type".to_string());
    };
    if segment.ident == "Vec" {
        let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
            return Err("Vec needs one type argument".to_string());
        };
        let Some(syn::GenericArgument::Type(inner)) = args.args.first() else {
            return Err("Vec needs one type argument".to_string());
        };
        return match classify_value(inner)? {
            ValueKind::Int64 => Ok(ValueKind::ListInt64),
            ValueKind::Utf8 => Ok(ValueKind::ListUtf8),
            other => Err(format!(
                "catalog lists support i64 and String elements, not {}",
                other.sql_name()
            )),
        };
    }
    if !matches!(segment.arguments, syn::PathArguments::None) {
        return Err(format!("cannot build a catalog batch for {}", segment.ident));
    }
    let kind = match segment.ident.to_string().as_str() {
        "bool" => ValueKind::Boolean,
        "i64" | "u64" => ValueKind::Int64,
        "i32" | "u32" => ValueKind::Int32,
        "i16" | "u16" | "i8" | "u8" => ValueKind::Int16,
        "f64" => ValueKind::Float64,
        "f32" => ValueKind::Float32,
        "String" => ValueKind::Utf8,
        "Timestamp" => ValueKind::Timestamp,
        "Value" | "Json" => ValueKind::Json,
        other if other.ends_with("Id") || other == "TableName" || other == "NodeId" => {
            ValueKind::Utf8
        },
        other => return Err(format!("cannot build a catalog batch for {other}")),
    };
    Ok(kind)
}

fn align_value_kind(kind: ValueKind, sql_variant: &str) -> std::result::Result<ValueKind, String> {
    match (kind, sql_variant) {
        (ValueKind::Int64, "Timestamp") => Ok(ValueKind::Timestamp),
        (ValueKind::Utf8, "Json") => Ok(ValueKind::Json),
        (ValueKind::ListInt64 | ValueKind::ListUtf8, "Json") => Ok(kind),
        (kind, variant) if kind.sql_name() == variant || variant.is_empty() => Ok(kind),
        (kind, variant) => Err(format!(
            "SQL type {variant} does not match the field type ({})",
            kind.sql_name()
        )),
    }
}

fn arrow_schema_fn(columns: &[ColumnSpec], schema_error: &str) -> proc_macro2::TokenStream {
    let list_arms = columns.iter().enumerate().filter_map(|(index, spec)| {
        let data_type = match spec.args.value_kind? {
            ValueKind::ListInt64 => quote!(arrow::datatypes::DataType::Int64),
            ValueKind::ListUtf8 => quote!(arrow::datatypes::DataType::Utf8),
            _ => return None,
        };
        Some(quote! {
            #index => {
                return std::sync::Arc::new(arrow::datatypes::Field::new(
                    field.name().clone(),
                    arrow::datatypes::DataType::List(std::sync::Arc::new(
                        arrow::datatypes::Field::new("item", #data_type, true),
                    )),
                    field.is_nullable(),
                ));
            }
        })
    });
    let list_arms: Vec<_> = list_arms.collect();
    if list_arms.is_empty() {
        quote! {
            pub fn arrow_schema() -> kalamdb_commons::schemas::TableArrowSchema {
                static SCHEMA: std::sync::OnceLock<kalamdb_commons::schemas::TableArrowSchema> =
                    std::sync::OnceLock::new();
                SCHEMA
                    .get_or_init(|| Self::definition().to_arrow_schema().expect(#schema_error))
                    .clone()
            }
        }
    } else {
        quote! {
            pub fn arrow_schema() -> kalamdb_commons::schemas::TableArrowSchema {
                static SCHEMA: std::sync::OnceLock<kalamdb_commons::schemas::TableArrowSchema> =
                    std::sync::OnceLock::new();
                SCHEMA.get_or_init(|| {
                    let schema = Self::definition().to_arrow_schema().expect(#schema_error);
                    let fields: Vec<std::sync::Arc<arrow::datatypes::Field>> = schema
                        .fields()
                        .iter()
                        .enumerate()
                        .map(|(index, field)| match index {
                            #(#list_arms)*
                            _ => std::sync::Arc::clone(field),
                        })
                        .collect();
                    std::sync::Arc::new(arrow::datatypes::Schema::new_with_metadata(
                        fields,
                        schema.metadata().clone(),
                    ))
                })
                .clone()
            }
        }
    }
}

fn catalog_row_impl(struct_ident: &Ident, columns: &[ColumnSpec]) -> proc_macro2::TokenStream {
    let mut inits = Vec::new();
    let mut appends = Vec::new();
    let mut finishes = Vec::new();
    for (index, spec) in columns.iter().enumerate() {
        let kind = spec.args.value_kind.expect("value kind is set in complete()");
        let builder = Ident::new(&format!("column_{index}"), proc_macro2::Span::call_site());
        let field = &spec.field_name;
        let init = builder_new(kind);
        inits.push(quote! { let mut #builder = #init; });
        appends.push(append_value(kind, spec.args.is_nullable, &builder, field));
        finishes.push(quote! {
            std::sync::Arc::new(#builder.finish()) as arrow::array::ArrayRef
        });
    }

    quote! {
        impl #struct_ident {
            /// Pack rows into one batch. The schema is [`Self::arrow_schema`].
            pub fn to_batch(rows: &[Self]) -> arrow::record_batch::RecordBatch {
                <Self as kalamdb_commons::schemas::CatalogRow>::to_batch(rows)
            }
        }

        impl kalamdb_commons::schemas::CatalogRow for #struct_ident {
            fn batch_schema() -> std::sync::Arc<arrow::datatypes::Schema> {
                Self::arrow_schema()
            }

            fn to_batch(rows: &[Self]) -> arrow::record_batch::RecordBatch {
                #(#inits)*
                for row in rows {
                    #(#appends)*
                }
                arrow::record_batch::RecordBatch::try_new(
                    Self::arrow_schema(),
                    vec![#(#finishes),*],
                )
                .expect("catalog row batch matches its schema")
            }
        }
    }
}

fn builder_new(kind: ValueKind) -> proc_macro2::TokenStream {
    match kind {
        ValueKind::Boolean => quote!(arrow::array::BooleanBuilder::new()),
        ValueKind::Int16 => quote!(arrow::array::Int16Builder::new()),
        ValueKind::Int32 => quote!(arrow::array::Int32Builder::new()),
        ValueKind::Int64 => quote!(arrow::array::Int64Builder::new()),
        ValueKind::Float32 => quote!(arrow::array::Float32Builder::new()),
        ValueKind::Float64 => quote!(arrow::array::Float64Builder::new()),
        ValueKind::Utf8 | ValueKind::Json => quote!(arrow::array::StringBuilder::new()),
        ValueKind::Timestamp => quote!(arrow::array::TimestampMicrosecondBuilder::new()),
        ValueKind::ListInt64 => {
            quote!(arrow::array::ListBuilder::new(arrow::array::Int64Builder::new()))
        },
        ValueKind::ListUtf8 => {
            quote!(arrow::array::ListBuilder::new(arrow::array::StringBuilder::new()))
        },
    }
}

fn append_value(
    kind: ValueKind,
    nullable: bool,
    builder: &Ident,
    field: &Ident,
) -> proc_macro2::TokenStream {
    let value = quote!(row.#field);
    match (kind, nullable) {
        (ValueKind::Utf8, false) => quote!(#builder.append_value(&#value);),
        (ValueKind::Utf8, true) => quote! {
            match &#value {
                Some(value) => #builder.append_value(value),
                None => #builder.append_null(),
            }
        },
        (ValueKind::Json, false) => quote! {
            #builder.append_value(
                &serde_json::to_string(&#value).unwrap_or_else(|_| "null".to_string()),
            );
        },
        (ValueKind::Json, true) => quote! {
            match &#value {
                Some(value) => #builder.append_value(
                    &serde_json::to_string(value).unwrap_or_else(|_| "null".to_string()),
                ),
                None => #builder.append_null(),
            }
        },
        (ValueKind::ListInt64, false) => quote! {
            for item in &#value {
                #builder.values().append_value(*item);
            }
            #builder.append(true);
        },
        (ValueKind::ListInt64, true) => quote! {
            match &#value {
                Some(items) => {
                    for item in items {
                        #builder.values().append_value(*item);
                    }
                    #builder.append(true);
                },
                None => #builder.append(false),
            }
        },
        (ValueKind::ListUtf8, false) => quote! {
            for item in &#value {
                #builder.values().append_value(item);
            }
            #builder.append(true);
        },
        (ValueKind::ListUtf8, true) => quote! {
            match &#value {
                Some(items) => {
                    for item in items {
                        #builder.values().append_value(item);
                    }
                    #builder.append(true);
                },
                None => #builder.append(false),
            }
        },
        (_, false) => quote!(#builder.append_value(#value);),
        (_, true) => quote! {
            match #value {
                Some(value) => #builder.append_value(value),
                None => #builder.append_null(),
            }
        },
    }
}
