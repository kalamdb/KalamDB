//! Empty `pg_catalog` table schemas for PostgreSQL GUI probes that do not
//! correspond to persisted KalamDB catalog entities (`pg_inherits`, `pg_matviews`, etc.).
//!
//! Populated catalog tables (`pg_attrdef`, `pg_description`, `pg_enum`,
//! `pg_settings`, `pg_roles`, `pg_authid`, `pg_language`) are served by live
//! `kalamdb-views` providers backed by authoritative `system.*` stores.

use datafusion::arrow::datatypes::SchemaRef;

/// Number of empty `pg_catalog` tables registered for wire client probes.
pub const EMPTY_PG_CATALOG_TABLE_COUNT: usize = 12;

macro_rules! empty_pg_row {
    ($struct_name:ident, $table:literal, $($field:ident : $ty:ty),* $(,)?) => {
        #[kalamdb_macros::table(
            name = $table,
            namespace = SchemaId::pg_catalog(),
            view = true,
            comment = "Empty pg_catalog probe table"
        )]
        #[derive(Debug)]
        struct $struct_name {
            $(
                #[doc = $table]
                pub $field: $ty,
            )*
        }
    };
}

empty_pg_row!(PgInheritsRow, "pg_inherits", inhrelid: i64, inhparent: i64, inhseqno: i64);
empty_pg_row!(
    PgMatviewsRow,
    "pg_matviews",
    schemaname: String,
    matviewname: String,
    matviewowner: String,
    tablespace: Option<String>,
    hasindexes: bool,
    ispopulated: bool,
    definition: Option<String>
);
empty_pg_row!(
    PgAuthMembersRow,
    "pg_auth_members",
    oid: i64,
    roleid: i64,
    member: i64,
    grantor: i64,
    admin_option: bool
);
empty_pg_row!(
    PgCollationRow,
    "pg_collation",
    oid: i64,
    collname: String,
    collnamespace: i64,
    collowner: i64,
    collprovider: Option<String>,
    collisdeterministic: Option<bool>,
    collencoding: Option<i64>,
    collcollate: Option<String>,
    collctype: Option<String>
);
empty_pg_row!(PgAmRow, "pg_am", oid: i64, amname: String, amhandler: Option<i64>, amtype: Option<String>);
empty_pg_row!(
    PgCastRow,
    "pg_cast",
    oid: i64,
    castsource: i64,
    casttarget: i64,
    castfunc: Option<i64>,
    castcontext: Option<String>,
    castmethod: Option<String>
);
empty_pg_row!(
    PgDependRow,
    "pg_depend",
    classid: i64,
    objid: i64,
    objsubid: i64,
    refclassid: i64,
    refobjid: i64,
    refobjsubid: i64,
    deptype: String
);
empty_pg_row!(
    PgTablespaceRow,
    "pg_tablespace",
    oid: i64,
    spcname: String,
    spcowner: i64,
    spcacl: Option<String>,
    spcoptions: Option<String>
);
empty_pg_row!(
    PgTriggerRow,
    "pg_trigger",
    oid: i64,
    tgrelid: i64,
    tgparentid: Option<i64>,
    tgname: String,
    tgfoid: Option<i64>,
    tgtype: Option<i64>,
    tgenabled: Option<bool>,
    tgisinternal: Option<bool>
);
empty_pg_row!(
    PgExtensionRow,
    "pg_extension",
    oid: i64,
    extname: String,
    extowner: Option<i64>,
    extnamespace: Option<i64>,
    extrelocatable: Option<bool>,
    extversion: Option<String>
);
empty_pg_row!(
    PgRangeRow,
    "pg_range",
    rngtypid: i64,
    rngsubtype: i64,
    rngmultitypid: Option<i64>,
    rngcollation: Option<i64>,
    rngsubopc: Option<i64>,
    rngcanonical: Option<i64>,
    rngsubdiff: Option<i64>
);
empty_pg_row!(
    PgSequenceRow,
    "pg_sequence",
    seqrelid: i64,
    seqtypid: Option<i64>,
    seqstart: Option<i64>,
    seqincrement: Option<i64>,
    seqmax: Option<i64>,
    seqmin: Option<i64>,
    seqcache: Option<i64>,
    seqcycle: Option<bool>
);

/// Names of empty `pg_catalog` tables (for coverage tests).
pub fn empty_pg_catalog_table_names() -> &'static [&'static str] {
    &[
        "pg_inherits",
        "pg_matviews",
        "pg_auth_members",
        "pg_collation",
        "pg_am",
        "pg_cast",
        "pg_depend",
        "pg_tablespace",
        "pg_trigger",
        "pg_extension",
        "pg_range",
        "pg_sequence",
    ]
}

/// Arrow schemas for empty `pg_catalog` probe tables.
pub fn empty_pg_catalog_table_defs() -> Vec<(&'static str, SchemaRef)> {
    vec![
        ("pg_inherits", PgInheritsRow::arrow_schema()),
        ("pg_matviews", PgMatviewsRow::arrow_schema()),
        ("pg_auth_members", PgAuthMembersRow::arrow_schema()),
        ("pg_collation", PgCollationRow::arrow_schema()),
        ("pg_am", PgAmRow::arrow_schema()),
        ("pg_cast", PgCastRow::arrow_schema()),
        ("pg_depend", PgDependRow::arrow_schema()),
        ("pg_tablespace", PgTablespaceRow::arrow_schema()),
        ("pg_trigger", PgTriggerRow::arrow_schema()),
        ("pg_extension", PgExtensionRow::arrow_schema()),
        ("pg_range", PgRangeRow::arrow_schema()),
        ("pg_sequence", PgSequenceRow::arrow_schema()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_table_count_matches_names_and_defs() {
        assert_eq!(empty_pg_catalog_table_names().len(), EMPTY_PG_CATALOG_TABLE_COUNT);
        assert_eq!(empty_pg_catalog_table_defs().len(), EMPTY_PG_CATALOG_TABLE_COUNT);
        let names: Vec<_> =
            empty_pg_catalog_table_defs().into_iter().map(|(name, _)| name).collect();
        assert_eq!(names, empty_pg_catalog_table_names());
    }

    #[test]
    fn populated_catalog_tables_are_not_registered_as_empty_stubs() {
        let names = empty_pg_catalog_table_names();
        for populated in [
            "pg_settings",
            "pg_roles",
            "pg_authid",
            "pg_description",
            "pg_attrdef",
            "pg_enum",
            "pg_language",
            "pg_namespace",
            "pg_class",
            "pg_attribute",
            "pg_type",
            "pg_proc",
            "pg_constraint",
            "pg_index",
        ] {
            assert!(
                !names.contains(&populated),
                "populated catalog view {populated} must not be served from empty_tables"
            );
        }
    }
}
