//! Live named-type e2e: nested composites, arrays, enums, rename, and evolution.
//!
//! Complements `functions.rs` (`kobj_functions_create_type_and_inline_procedure`),
//! which already covers CREATE TYPE, procedure/REST round-trips, duplicate/empty
//! enum labels, unknown types, invalid CALL labels, ADD VALUE append, and DROP
//! TYPE with dependents.

use std::collections::HashMap;

use serde_json::Value;

use crate::{common::*, kobj_helpers::*};

fn type_rows(namespace: &str, name: &str) -> Vec<HashMap<String, Value>> {
    query_rows(&format!(
        "SELECT type_id, namespace_id, name, kind, type_revision, next_slot, comment FROM \
         system.types WHERE namespace_id = '{namespace}' AND name = '{name}'"
    ))
}

fn type_row(namespace: &str, name: &str) -> HashMap<String, Value> {
    let rows = type_rows(namespace, name);
    assert_eq!(rows.len(), 1, "expected one catalog type {namespace}.{name}, got {rows:?}");
    rows.into_iter().next().expect("type row")
}

fn type_id_of(namespace: &str, name: &str) -> String {
    cell_str(&type_row(namespace, name), "type_id").expect("type_id")
}

fn type_fields(type_id: &str) -> Vec<HashMap<String, Value>> {
    query_rows(&format!(
        "SELECT name, ordinal, slot, dropped, is_array, type_name FROM system.type_fields WHERE \
         type_id = '{type_id}' ORDER BY ordinal"
    ))
}

fn live_field_names(type_id: &str) -> Vec<String> {
    type_fields(type_id)
        .into_iter()
        .filter(|row| cell_bool(row, "dropped") != Some(true))
        .filter_map(|row| cell_str(&row, "name"))
        .collect()
}

fn jsonish(row: &HashMap<String, Value>, column: &str) -> String {
    match cell(row, column) {
        Value::Null => String::new(),
        Value::String(text) => text,
        other => other.to_string(),
    }
}

fn create_shared(full: &str, columns: &str) {
    exec(&format!("CREATE TABLE {full} ({columns}) WITH (TYPE = 'SHARED')"));
    grant_public_shared_table_access(full);
    ready(full);
}

#[ntest::timeout(180000)]
#[test]
fn kobj_type_nested_composite_table_round_trip() {
    if skip_if_no_server() {
        return;
    }
    let ns = setup_namespace("kobj_tnest");
    let table = generate_unique_table("people");
    let full = format!("{ns}.{table}");

    exec(&format!(
        "CREATE TYPE {ns}.address AS (city TEXT NOT NULL, country TEXT NOT NULL)"
    ));
    exec(&format!(
        "CREATE TYPE {ns}.contact AS (display TEXT NOT NULL, home {ns}.address)"
    ));
    create_shared(&full, &format!("id INT PRIMARY KEY, profile {ns}.contact"));

    exec(&format!(
        "INSERT INTO {full} (id, profile) VALUES (1, \
         '{{\"display\":\"Ada\",\"home\":{{\"city\":\"Paris\",\"country\":\"FR\"}}}}')"
    ));
    let row = &query_rows(&format!("SELECT profile FROM {full} WHERE id = 1"))[0];
    let profile = jsonish(row, "profile");
    assert!(profile.contains("Ada"), "nested display missing: {profile}");
    assert!(profile.contains("Paris"), "nested city missing: {profile}");
    assert!(profile.contains("FR"), "nested country missing: {profile}");

    let fields = type_fields(&type_id_of(&ns, "contact"));
    assert!(
        fields.iter().any(|row| {
            cell_str(row, "name").as_deref() == Some("home")
                && cell_str(row, "type_name").is_some_and(|name| name.contains("address"))
        }),
        "contact.home should reference address: {fields:?}"
    );
}

#[ntest::timeout(180000)]
#[test]
fn kobj_type_array_columns_scalar_and_named() {
    if skip_if_no_server() {
        return;
    }
    let ns = setup_namespace("kobj_tarr");
    let table = generate_unique_table("places");
    let full = format!("{ns}.{table}");

    exec(&format!(
        "CREATE TYPE {ns}.address AS (city TEXT NOT NULL, country TEXT NOT NULL)"
    ));
    create_shared(&full, &format!("id INT PRIMARY KEY, tags TEXT[], homes {ns}.address[]"));

    exec(&format!(
        "INSERT INTO {full} (id, tags, homes) VALUES (1, '[\"home\",\"work\"]', \
         '[{{\"city\":\"Paris\",\"country\":\"FR\"}},{{\"city\":\"Lyon\",\"country\":\"FR\"}}]')"
    ));
    let row = &query_rows(&format!("SELECT tags, homes FROM {full} WHERE id = 1"))[0];
    let tags = jsonish(row, "tags");
    let homes = jsonish(row, "homes");
    assert!(
        tags.contains("home") && tags.contains("work"),
        "scalar array missing labels: {tags}"
    );
    assert!(
        homes.contains("Paris") && homes.contains("Lyon"),
        "named-type array missing cities: {homes}"
    );
}

#[ntest::timeout(180000)]
#[test]
fn kobj_type_enum_column_add_value_before_after() {
    if skip_if_no_server() {
        return;
    }
    let ns = setup_namespace("kobj_tenum");
    let table = generate_unique_table("tickets");
    let full = format!("{ns}.{table}");

    exec(&format!("CREATE TYPE {ns}.status AS ENUM ('active', 'blocked')"));
    create_shared(&full, &format!("id INT PRIMARY KEY, status {ns}.status NOT NULL"));

    exec(&format!("INSERT INTO {full} (id, status) VALUES (1, 'active')"));
    let bad = exec_err(&format!("INSERT INTO {full} (id, status) VALUES (2, 'nope')"));
    let bad_lower = bad.to_ascii_lowercase();
    assert!(
        bad_lower.contains("invalid") || bad_lower.contains("enum") || bad_lower.contains("status"),
        "invalid enum insert must fail: {bad}"
    );

    exec(&format!("ALTER TYPE {ns}.status ADD VALUE 'pending' BEFORE 'active'"));
    exec(&format!("ALTER TYPE {ns}.status ADD VALUE 'archived' AFTER 'blocked'"));
    exec(&format!(
        "INSERT INTO {full} (id, status) VALUES (3, 'pending'), (4, 'archived')"
    ));

    let labels = live_field_names(&type_id_of(&ns, "status"));
    assert_eq!(
        labels,
        vec![
            "pending".to_string(),
            "active".to_string(),
            "blocked".to_string(),
            "archived".to_string()
        ],
        "ADD VALUE BEFORE/AFTER must keep declaration order: {labels:?}"
    );
    assert_eq!(
        count_sql(&format!("SELECT COUNT(*) AS n FROM {full}")),
        3,
        "valid enum labels including new neighbors must insert"
    );
}

#[ntest::timeout(180000)]
#[test]
fn kobj_type_rename_preserves_type_id_and_stored_rows() {
    if skip_if_no_server() {
        return;
    }
    let ns = setup_namespace("kobj_tren");
    let table = generate_unique_table("homes");
    let full = format!("{ns}.{table}");

    exec(&format!(
        "CREATE TYPE {ns}.address AS (city TEXT NOT NULL, country TEXT NOT NULL)"
    ));
    let original_id = type_id_of(&ns, "address");
    assert!(
        !original_id.contains('.'),
        "live TypeId must be opaque, not schema.name: {original_id}"
    );
    create_shared(&full, &format!("id INT PRIMARY KEY, home {ns}.address"));
    exec(&format!(
        "INSERT INTO {full} (id, home) VALUES (1, '{{\"city\":\"Paris\",\"country\":\"FR\"}}')"
    ));

    exec(&format!("ALTER TYPE {ns}.address RENAME TO user_address"));
    assert!(type_rows(&ns, "address").is_empty(), "old alias must disappear after RENAME TO");
    let renamed = type_row(&ns, "user_address");
    assert_eq!(cell_str(&renamed, "type_id").as_deref(), Some(original_id.as_str()));
    assert_eq!(cell_str(&renamed, "name").as_deref(), Some("user_address"));

    let stored = jsonish(&query_rows(&format!("SELECT home FROM {full} WHERE id = 1"))[0], "home");
    assert!(stored.contains("Paris"), "rows must survive type rename: {stored}");

    let stale = exec_err(&format!(
        "CREATE TABLE {ns}.stale (id INT PRIMARY KEY, home {ns}.address) WITH (TYPE = 'SHARED')"
    ));
    assert!(
        stale.to_ascii_lowercase().contains("not found")
            || stale.to_ascii_lowercase().contains("address"),
        "old type name must not resolve after rename: {stale}"
    );

    let relocated = format!("{ns}.{}", generate_unique_table("moved"));
    create_shared(&relocated, &format!("id INT PRIMARY KEY, home {ns}.user_address"));
    exec(&format!(
        "INSERT INTO {relocated} (id, home) VALUES (1, '{{\"city\":\"Lyon\",\"country\":\"FR\"}}')"
    ));
    let moved =
        jsonish(&query_rows(&format!("SELECT home FROM {relocated} WHERE id = 1"))[0], "home");
    assert!(moved.contains("Lyon"), "new alias must bind to the same TypeId: {moved}");
}

#[ntest::timeout(180000)]
#[test]
fn kobj_type_set_schema_moves_alias_only() {
    if skip_if_no_server() {
        return;
    }
    let src = setup_namespace("kobj_tseta");
    let dest = setup_namespace("kobj_tsetb");

    exec(&format!("CREATE TYPE {src}.address AS (city TEXT NOT NULL)"));
    let original_id = type_id_of(&src, "address");

    exec(&format!("ALTER TYPE {src}.address SET SCHEMA {dest}"));
    assert!(
        type_rows(&src, "address").is_empty(),
        "source alias must be gone after SET SCHEMA"
    );
    let moved = type_row(&dest, "address");
    assert_eq!(cell_str(&moved, "type_id").as_deref(), Some(original_id.as_str()));
    assert_eq!(cell_str(&moved, "namespace_id").as_deref(), Some(dest.as_str()));
}

#[ntest::timeout(180000)]
#[test]
fn kobj_type_add_attribute_nullable_evolution() {
    if skip_if_no_server() {
        return;
    }
    let ns = setup_namespace("kobj_tevo");
    let table = generate_unique_table("homes");
    let full = format!("{ns}.{table}");

    exec(&format!("CREATE TYPE {ns}.address AS (city TEXT NOT NULL)"));
    create_shared(&full, &format!("id INT PRIMARY KEY, home {ns}.address"));
    exec(&format!("INSERT INTO {full} (id, home) VALUES (1, '{{\"city\":\"Paris\"}}')"));
    let before = cell_i64(&type_row(&ns, "address"), "type_revision").unwrap_or(1);

    let not_null = exec_err(&format!("ALTER TYPE {ns}.address ADD ATTRIBUTE zip TEXT NOT NULL"));
    assert!(
        not_null.to_ascii_lowercase().contains("not null")
            || not_null.to_ascii_lowercase().contains("dependent"),
        "NOT NULL append must be rejected while the type is stored: {not_null}"
    );

    exec(&format!("ALTER TYPE {ns}.address ADD ATTRIBUTE zip TEXT"));
    let after = type_row(&ns, "address");
    assert!(
        cell_i64(&after, "type_revision").unwrap_or(0) > before,
        "nullable append must bump type_revision: {after:?}"
    );
    assert_eq!(
        live_field_names(&type_id_of(&ns, "address")),
        vec!["city".to_string(), "zip".to_string()]
    );

    let old = jsonish(&query_rows(&format!("SELECT home FROM {full} WHERE id = 1"))[0], "home");
    assert!(old.contains("Paris"), "pre-evolution city must remain: {old}");
    assert!(!old.contains("75001"), "old row must not invent the new attribute: {old}");

    exec(&format!(
        "INSERT INTO {full} (id, home) VALUES (2, '{{\"city\":\"Lyon\",\"zip\":\"69001\"}}')"
    ));
    let new = jsonish(&query_rows(&format!("SELECT home FROM {full} WHERE id = 2"))[0], "home");
    assert!(
        new.contains("Lyon") && new.contains("69001"),
        "new rows must write the appended field: {new}"
    );
}

#[ntest::timeout(180000)]
#[test]
fn kobj_type_drop_attribute_blocked_when_stored() {
    if skip_if_no_server() {
        return;
    }
    let ns = setup_namespace("kobj_tdrop");
    let table = generate_unique_table("homes");
    let full = format!("{ns}.{table}");

    exec(&format!(
        "CREATE TYPE {ns}.address AS (city TEXT NOT NULL, country TEXT NOT NULL)"
    ));
    create_shared(&full, &format!("id INT PRIMARY KEY, home {ns}.address"));
    exec(&format!(
        "INSERT INTO {full} (id, home) VALUES (1, '{{\"city\":\"Paris\",\"country\":\"FR\"}}')"
    ));

    let drop_attr = exec_err(&format!("ALTER TYPE {ns}.address DROP ATTRIBUTE country"));
    assert!(
        drop_attr.to_ascii_lowercase().contains("dependent")
            || drop_attr.to_ascii_lowercase().contains("stored")
            || drop_attr.to_ascii_lowercase().contains("referenced"),
        "DROP ATTRIBUTE must fail while a table stores the type: {drop_attr}"
    );
    assert_eq!(
        live_field_names(&type_id_of(&ns, "address")),
        vec!["city".to_string(), "country".to_string()],
        "blocked DROP ATTRIBUTE must not tombstone live fields"
    );
}

#[ntest::timeout(180000)]
#[test]
fn kobj_type_drop_create_same_name_allocates_new_type_id() {
    if skip_if_no_server() {
        return;
    }
    let ns = setup_namespace("kobj_treid");

    exec(&format!("CREATE TYPE {ns}.address AS (city TEXT NOT NULL)"));
    let first = type_id_of(&ns, "address");
    exec(&format!("DROP TYPE {ns}.address"));
    assert!(type_rows(&ns, "address").is_empty(), "DROP TYPE must remove the catalog alias");

    exec(&format!(
        "CREATE TYPE {ns}.address AS (city TEXT NOT NULL, country TEXT NOT NULL)"
    ));
    let second = type_id_of(&ns, "address");
    assert_ne!(first, second, "DROP + CREATE of the same name must allocate a new TypeId");
    assert_eq!(live_field_names(&second), vec!["city".to_string(), "country".to_string()]);
}

#[ntest::timeout(180000)]
#[test]
fn kobj_type_self_reference_and_cycle_rejected() {
    if skip_if_no_server() {
        return;
    }
    let ns = setup_namespace("kobj_tcyc");

    let self_ref = exec_err(&format!("CREATE TYPE {ns}.node AS (child {ns}.node)"));
    assert!(
        self_ref.to_ascii_lowercase().contains("itself")
            || self_ref.to_ascii_lowercase().contains("cyclic"),
        "self-referential composite must fail: {self_ref}"
    );

    exec(&format!("CREATE TYPE {ns}.alpha AS (label TEXT NOT NULL)"));
    exec(&format!("CREATE TYPE {ns}.beta AS (parent {ns}.alpha)"));
    let cycle = exec_err(&format!("ALTER TYPE {ns}.alpha ADD ATTRIBUTE child {ns}.beta"));
    assert!(
        cycle.to_ascii_lowercase().contains("cyclic")
            || cycle.to_ascii_lowercase().contains("cycle"),
        "indirect type cycle must fail: {cycle}"
    );
}

#[ntest::timeout(180000)]
#[test]
fn kobj_type_comment_and_drop_attribute_tombstone_without_dependents() {
    if skip_if_no_server() {
        return;
    }
    let ns = setup_namespace("kobj_ttomb");

    exec(&format!(
        "CREATE TYPE {ns}.address AS (city TEXT NOT NULL, country TEXT NOT NULL)"
    ));
    exec(&format!("COMMENT ON TYPE {ns}.address IS 'Postal address'"));
    let commented = type_row(&ns, "address");
    assert_eq!(cell_str(&commented, "comment").as_deref(), Some("Postal address"));

    let type_id = cell_str(&commented, "type_id").expect("type_id");
    let before_fields = type_fields(&type_id);
    let country = before_fields
        .iter()
        .find(|row| cell_str(row, "name").as_deref() == Some("country"))
        .expect("country field");
    let country_slot = cell_i64(country, "slot").unwrap_or(0);

    exec(&format!("ALTER TYPE {ns}.address DROP ATTRIBUTE country"));
    let after_fields = type_fields(&type_id);
    let tombstone = after_fields
        .iter()
        .find(|row| cell_str(row, "name").as_deref() == Some("country"))
        .expect("tombstoned country");
    assert_eq!(cell_bool(tombstone, "dropped"), Some(true));
    assert_eq!(cell_i64(tombstone, "slot").unwrap_or(0), country_slot);

    exec(&format!("ALTER TYPE {ns}.address ADD ATTRIBUTE zip TEXT"));
    let zip = type_fields(&type_id)
        .into_iter()
        .find(|row| cell_str(row, "name").as_deref() == Some("zip"))
        .expect("zip field");
    let zip_slot = cell_i64(&zip, "slot").unwrap_or(0);
    assert!(
        zip_slot > country_slot,
        "new attribute must not reuse a tombstoned slot: country={country_slot} zip={zip_slot}"
    );
}
