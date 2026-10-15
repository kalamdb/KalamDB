//! Empty `pg_catalog` table schemas for PostgreSQL GUI probes that do not
//! correspond to persisted KalamDB catalog entities (`pg_inherits`, `pg_matviews`, etc.).
//!
//! Populated catalog tables (`pg_attrdef`, `pg_description`, `pg_enum`,
//! `pg_settings`, `pg_roles`, `pg_authid`, `pg_language`) are served by live
//! `kalamdb-views` providers backed by authoritative `system.*` stores.
//!
//! Also compiled into `kalamdb-views` via `#[path]` so providers and wire share
//! one definition without a crate dependency cycle.

use datafusion::arrow::datatypes::{DataType, Field};

/// Number of empty `pg_catalog` tables registered for wire client probes.
pub const EMPTY_PG_CATALOG_TABLE_COUNT: usize = 12;

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

fn int64(name: &str, nullable: bool) -> Field {
    Field::new(name, DataType::Int64, nullable)
}

fn utf8(name: &str, nullable: bool) -> Field {
    Field::new(name, DataType::Utf8, nullable)
}

fn bool_field(name: &str, nullable: bool) -> Field {
    Field::new(name, DataType::Boolean, nullable)
}

/// Schema definitions for empty `pg_catalog` probe tables.
pub fn empty_pg_catalog_table_defs() -> Vec<(&'static str, Vec<Field>)> {
    vec![
        (
            "pg_inherits",
            vec![
                int64("inhrelid", false),
                int64("inhparent", false),
                int64("inhseqno", false),
            ],
        ),
        (
            "pg_matviews",
            vec![
                utf8("schemaname", false),
                utf8("matviewname", false),
                utf8("matviewowner", false),
                utf8("tablespace", true),
                bool_field("hasindexes", false),
                bool_field("ispopulated", false),
                utf8("definition", true),
            ],
        ),
        (
            "pg_auth_members",
            vec![
                int64("oid", false),
                int64("roleid", false),
                int64("member", false),
                int64("grantor", false),
                bool_field("admin_option", false),
            ],
        ),
        (
            "pg_collation",
            vec![
                int64("oid", false),
                utf8("collname", false),
                int64("collnamespace", false),
                int64("collowner", false),
                utf8("collprovider", true),
                bool_field("collisdeterministic", true),
                int64("collencoding", true),
                utf8("collcollate", true),
                utf8("collctype", true),
            ],
        ),
        (
            "pg_am",
            vec![
                int64("oid", false),
                utf8("amname", false),
                int64("amhandler", true),
                utf8("amtype", true),
            ],
        ),
        (
            "pg_cast",
            vec![
                int64("oid", false),
                int64("castsource", false),
                int64("casttarget", false),
                int64("castfunc", true),
                utf8("castcontext", true),
                utf8("castmethod", true),
            ],
        ),
        (
            "pg_depend",
            vec![
                int64("classid", false),
                int64("objid", false),
                int64("objsubid", false),
                int64("refclassid", false),
                int64("refobjid", false),
                int64("refobjsubid", false),
                utf8("deptype", false),
            ],
        ),
        (
            "pg_tablespace",
            vec![
                int64("oid", false),
                utf8("spcname", false),
                int64("spcowner", false),
                utf8("spcacl", true),
                utf8("spcoptions", true),
            ],
        ),
        (
            "pg_trigger",
            vec![
                int64("oid", false),
                int64("tgrelid", false),
                int64("tgparentid", true),
                utf8("tgname", false),
                int64("tgfoid", true),
                int64("tgtype", true),
                bool_field("tgenabled", true),
                bool_field("tgisinternal", true),
            ],
        ),
        (
            "pg_extension",
            vec![
                int64("oid", false),
                utf8("extname", false),
                int64("extowner", true),
                int64("extnamespace", true),
                bool_field("extrelocatable", true),
                utf8("extversion", true),
            ],
        ),
        (
            "pg_range",
            vec![
                int64("rngtypid", false),
                int64("rngsubtype", false),
                int64("rngmultitypid", true),
                int64("rngcollation", true),
                int64("rngsubopc", true),
                int64("rngcanonical", true),
                int64("rngsubdiff", true),
            ],
        ),
        (
            "pg_sequence",
            vec![
                int64("seqrelid", false),
                int64("seqtypid", true),
                int64("seqstart", true),
                int64("seqincrement", true),
                int64("seqmax", true),
                int64("seqmin", true),
                int64("seqcache", true),
                bool_field("seqcycle", true),
            ],
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_table_count_matches_names_and_defs() {
        assert_eq!(empty_pg_catalog_table_names().len(), EMPTY_PG_CATALOG_TABLE_COUNT);
        assert_eq!(empty_pg_catalog_table_defs().len(), EMPTY_PG_CATALOG_TABLE_COUNT);
        let names: Vec<_> = empty_pg_catalog_table_defs().into_iter().map(|(n, _)| n).collect();
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
