use kalam_schema_diff::{
    diff_schema_sql, diff_schema_sql_with_options, DiffOptions, MigrationStatements,
};

fn diff(before: &str, after: &str) -> MigrationStatements {
    diff_schema_sql(before, after).expect("diff schemas")
}

fn destructive_diff(before: &str, after: &str) -> MigrationStatements {
    diff_schema_sql_with_options(
        before,
        after,
        &DiffOptions {
            allow_destructive: true,
        },
    )
    .expect("diff schemas")
}

fn assert_contains(haystack: &str, needle: &str) {
    assert!(haystack.contains(needle), "expected to find:\n{needle}\n\nin:\n{haystack}");
}

fn assert_not_contains(haystack: &str, needle: &str) {
    assert!(!haystack.contains(needle), "expected not to find:\n{needle}\n\nin:\n{haystack}");
}

#[test]
fn detect_create_table() {
    let diff = diff(
        "",
        r#"
            CREATE USER TABLE app.users (
              id BIGINT PRIMARY KEY,
              email TEXT NOT NULL
            );
        "#,
    );

    assert_contains(&diff.up, "CREATE USER TABLE app.users");
    assert_contains(&diff.up, "email TEXT NOT NULL");
}

#[test]
fn detect_create_namespaces_from_namespace_and_schema_syntax() {
    let diff = diff(
        "",
        r#"
            CREATE NAMESPACE app;
            CREATE SCHEMA IF NOT EXISTS analytics;
        "#,
    );

    assert_contains(&diff.up, "CREATE NAMESPACE IF NOT EXISTS analytics;");
    assert_contains(&diff.up, "CREATE NAMESPACE IF NOT EXISTS app;");
}

#[test]
fn detect_create_table_kinds_from_prefix_and_type_option() {
    let diff = diff(
        "",
        r#"
            CREATE USER TABLE app.users (id BIGINT PRIMARY KEY);
            CREATE SHARED TABLE app.shared_messages (id BIGINT PRIMARY KEY);
            CREATE STREAM TABLE app.message_streams (id BIGINT PRIMARY KEY);
            CREATE TABLE app.option_kind (id BIGINT PRIMARY KEY) WITH (TYPE = 'USER');
        "#,
    );

    assert_contains(&diff.up, "CREATE USER TABLE app.option_kind");
    assert_contains(&diff.up, "CREATE USER TABLE app.users");
    assert_contains(&diff.up, "CREATE SHARED TABLE app.shared_messages");
    assert_contains(&diff.up, "CREATE STREAM TABLE app.message_streams");
}

#[test]
fn detect_create_table_constraints_and_options() {
    let diff = diff(
        "",
        r#"
            CREATE TABLE app.events (
              id BIGINT,
              email TEXT NOT NULL,
              PRIMARY KEY (id)
            )
            WITH (
              STORAGE_ID = 'default',
              COMPRESSION = 'zstd'
            );
        "#,
    );

    assert_contains(&diff.up, "CREATE TABLE app.events");
    assert_contains(&diff.up, "PRIMARY KEY (id)");
    assert_contains(&diff.up, "COMPRESSION = 'zstd'");
    assert_contains(&diff.up, "STORAGE_ID = 'default'");
}

#[test]
fn detect_drop_table() {
    let diff = destructive_diff("CREATE TABLE app.users (id BIGINT PRIMARY KEY);", "");

    assert_contains(&diff.up, "DROP TABLE app.users;");
}

#[test]
fn detect_add_column() {
    let diff = diff(
        "CREATE TABLE app.users (id BIGINT PRIMARY KEY);",
        "CREATE TABLE app.users (id BIGINT PRIMARY KEY, email TEXT NOT NULL);",
    );

    assert_contains(&diff.up, "ALTER TABLE app.users ADD COLUMN email TEXT NOT NULL;");
}

#[test]
fn detect_drop_column() {
    let diff = destructive_diff(
        "CREATE TABLE app.users (id BIGINT PRIMARY KEY, email TEXT NOT NULL);",
        "CREATE TABLE app.users (id BIGINT PRIMARY KEY);",
    );

    assert_contains(&diff.up, "ALTER TABLE app.users DROP COLUMN email;");
}

#[test]
fn detect_drop_table_is_advisory_without_destructive_option() {
    let diff = diff("CREATE TABLE app.users (id BIGINT PRIMARY KEY);", "");

    assert_contains(
        &diff.up,
        "-- destructive change skipped: table app.users exists in current schema but not in target schema",
    );
    assert_contains(
        &diff.up,
        "-- rerun with destructive changes enabled to emit: DROP TABLE app.users;",
    );
    assert!(!diff.up.lines().any(|line| line == "DROP TABLE app.users;"), "{}", diff.up);
}

#[test]
fn detect_drop_column_is_advisory_without_destructive_option() {
    let diff = diff(
        "CREATE TABLE app.users (id BIGINT PRIMARY KEY, email TEXT NOT NULL);",
        "CREATE TABLE app.users (id BIGINT PRIMARY KEY);",
    );

    assert_contains(
        &diff.up,
        "-- destructive change skipped: column app.users.email exists in current schema but not in target schema",
    );
    assert_contains(
        &diff.up,
        "-- rerun with destructive changes enabled to emit: ALTER TABLE app.users DROP COLUMN email;",
    );
    assert!(
        !diff.up.lines().any(|line| line == "ALTER TABLE app.users DROP COLUMN email;"),
        "{}",
        diff.up
    );
}

#[test]
fn detect_modify_column_type() {
    let diff = diff(
        "CREATE TABLE app.users (id BIGINT PRIMARY KEY, age INT);",
        "CREATE TABLE app.users (id BIGINT PRIMARY KEY, age BIGINT);",
    );

    assert_contains(&diff.up, "ALTER TABLE app.users MODIFY COLUMN age BIGINT NULL;");
}

#[test]
fn detect_modify_column_nullability() {
    let diff = diff(
        "CREATE TABLE app.users (id BIGINT PRIMARY KEY, email TEXT);",
        "CREATE TABLE app.users (id BIGINT PRIMARY KEY, email TEXT NOT NULL);",
    );

    assert_contains(&diff.up, "ALTER TABLE app.users MODIFY COLUMN email TEXT NOT NULL;");
}

#[test]
fn detect_modify_column_default() {
    let diff = diff(
        "CREATE TABLE app.users (id BIGINT PRIMARY KEY, status TEXT DEFAULT 'pending');",
        "CREATE TABLE app.users (id BIGINT PRIMARY KEY, status TEXT DEFAULT 'active');",
    );

    assert_contains(
        &diff.up,
        "ALTER TABLE app.users MODIFY COLUMN status TEXT NULL DEFAULT 'active';",
    );
}

#[test]
fn detect_change_table_options() {
    let diff = diff(
        r#"
            CREATE TABLE app.events (id BIGINT PRIMARY KEY)
            WITH (COMPRESSION = 'snappy', STORAGE_ID = 'default');
        "#,
        r#"
            CREATE TABLE app.events (id BIGINT PRIMARY KEY)
            WITH (COMPRESSION = 'zstd', STORAGE_ID = 'default');
        "#,
    );

    assert_contains(&diff.up, "ALTER TABLE app.events SET TBLPROPERTIES (COMPRESSION = 'zstd');");
}

#[test]
fn detect_create_topic() {
    let diff = diff("", "CREATE TOPIC app.events PARTITIONS 4 WITH (retention_seconds = 3600);");

    assert_contains(
        &diff.up,
        "CREATE TOPIC app.events PARTITIONS 4 WITH (retention_seconds = 3600);",
    );
}

#[test]
fn detect_drop_topic() {
    let diff = destructive_diff("CREATE TOPIC app.events;", "");

    assert_contains(&diff.up, "DROP TOPIC app.events;");
}

#[test]
fn detect_change_topic_retention() {
    let diff = diff(
        "CREATE TOPIC app.events WITH (retention_seconds = 3600);",
        "CREATE TOPIC app.events WITH (retention_seconds = 7200, retention_max_bytes = NULL);",
    );

    assert_contains(
        &diff.up,
        "ALTER TOPIC app.events SET RETENTION WITH (retention_seconds = 7200, retention_max_bytes = NULL);",
    );
}

#[test]
fn detect_clear_topic_retention() {
    let diff = diff(
        "CREATE TOPIC app.events WITH (retention_seconds = 3600);",
        "CREATE TOPIC app.events;",
    );

    assert_contains(&diff.up, "ALTER TOPIC app.events CLEAR RETENTION;");
}

#[test]
fn detect_add_topic_source_route() {
    let before = r#"
        CREATE TABLE message_streams (id BIGINT PRIMARY KEY, body TEXT NOT NULL);
        CREATE TOPIC app.events;
    "#;
    let after = r#"
        CREATE TABLE message_streams (id BIGINT PRIMARY KEY, body TEXT NOT NULL);
        CREATE TOPIC app.events;
        ALTER TOPIC app.events ADD SOURCE message_streams ON INSERT WITH (payload = 'full');
    "#;

    let diff = diff(before, after);

    assert_contains(
        &diff.up,
        "ALTER TOPIC app.events ADD SOURCE message_streams ON INSERT WITH (payload = 'full');",
    );
    assert_not_contains(&diff.up, "CREATE TOPIC app.events;");
}

#[test]
fn detect_topic_if_not_exists_and_default_payload_route() {
    let diff = diff(
        "",
        r#"
            CREATE TABLE message_streams (id BIGINT PRIMARY KEY, body TEXT NOT NULL);
            CREATE TOPIC IF NOT EXISTS app.events;
            ALTER TOPIC app.events ADD SOURCE message_streams ON UPDATE;
        "#,
    );

    assert_contains(&diff.up, "CREATE TOPIC IF NOT EXISTS app.events;");
    assert_contains(&diff.up, "ALTER TOPIC app.events ADD SOURCE message_streams ON UPDATE;");
    assert_not_contains(
        &diff.up,
        "ALTER TOPIC app.events ADD SOURCE message_streams ON UPDATE WITH",
    );
}

#[test]
fn detect_drop_topic_source_route() {
    let before = r#"
        CREATE TABLE message_streams (id BIGINT PRIMARY KEY, body TEXT NOT NULL);
        CREATE TOPIC app.events;
        ALTER TOPIC app.events ADD SOURCE message_streams ON DELETE WITH (payload = 'key');
    "#;
    let after = r#"
        CREATE TABLE message_streams (id BIGINT PRIMARY KEY, body TEXT NOT NULL);
        CREATE TOPIC app.events;
    "#;

    let diff = diff(before, after);

    assert_contains(
        &diff.up,
        "-- manual review required: topic app.events source message_streams on DELETE was removed from target schema",
    );
}

#[test]
fn detect_change_topic_source_where_filter() {
    let before = r#"
        CREATE TABLE message_streams (id BIGINT PRIMARY KEY, priority INT, body TEXT NOT NULL);
        CREATE TOPIC app.events;
        ALTER TOPIC app.events ADD SOURCE message_streams ON INSERT WHERE priority >= 5 WITH (payload = 'full');
    "#;
    let after = r#"
        CREATE TABLE message_streams (id BIGINT PRIMARY KEY, priority INT, body TEXT NOT NULL);
        CREATE TOPIC app.events;
        ALTER TOPIC app.events ADD SOURCE message_streams ON INSERT WHERE priority >= 10 WITH (payload = 'full');
    "#;

    let diff = diff(before, after);

    assert_contains(
        &diff.up,
        "ALTER TOPIC app.events ADD SOURCE message_streams ON INSERT WHERE priority >= 10 WITH (payload = 'full');",
    );
    assert_contains(
        &diff.up,
        "-- manual review required: topic app.events source message_streams on INSERT was removed from target schema",
    );
}

#[test]
fn detect_noop_same_schema_with_different_formatting() {
    let before = r#"
        CREATE TABLE app.users (
          id BIGINT PRIMARY KEY,
          email TEXT NOT NULL
        )
        WITH (
          COMPRESSION = 'zstd'
        );
    "#;
    let after = "CREATE TABLE app.users(id BIGINT PRIMARY KEY,email TEXT NOT NULL) WITH (COMPRESSION='zstd');";

    let diff = diff(before, after);

    assert_contains(&diff.up, "-- No schema changes.");
}

#[test]
fn detect_schema_diff_order_is_deterministic() {
    let before = "";
    let after = r#"
        CREATE TABLE zeta (id BIGINT PRIMARY KEY);
        CREATE TOPIC app.zeta_events;
        CREATE TABLE alpha (id BIGINT PRIMARY KEY);
        CREATE NAMESPACE app;
        CREATE TOPIC app.alpha_events;
    "#;

    let first = diff(before, after);
    let second = diff(before, after);

    assert_eq!(first.up, second.up);
    assert!(
        first.up.find("CREATE NAMESPACE IF NOT EXISTS app;").unwrap()
            < first.up.find("CREATE TABLE alpha").unwrap()
    );
    assert!(
        first.up.find("CREATE TABLE alpha").unwrap() < first.up.find("CREATE TABLE zeta").unwrap()
    );
    assert!(
        first.up.find("CREATE TOPIC app.alpha_events;").unwrap()
            < first.up.find("CREATE TOPIC app.zeta_events;").unwrap()
    );
}

#[test]
fn detect_topic_source_missing_table_definition() {
    let err = diff_schema_sql(
        "",
        r#"
            CREATE TOPIC app.events;
            ALTER TOPIC app.events ADD SOURCE message_streams ON INSERT;
        "#,
    )
    .expect_err("topic source should require a table definition");
    let message = err.to_string();

    assert_contains(&message, "topic source table message_streams must be defined in schema.sql");
}

#[test]
fn combined_schema_diff_detects_most_cases() {
    let before = r#"
        CREATE NAMESPACE app;
        CREATE TABLE message_streams (
          id BIGINT PRIMARY KEY,
          body TEXT NOT NULL,
          priority INT DEFAULT 1,
          obsolete TEXT
        )
        WITH (COMPRESSION = 'snappy');
        CREATE TABLE app.old_table (id BIGINT PRIMARY KEY);
        CREATE TOPIC app.events WITH (retention_seconds = 3600);
        ALTER TOPIC app.events ADD SOURCE message_streams ON INSERT WHERE priority >= 5 WITH (payload = 'full');
        CREATE TOPIC app.old_topic;
    "#;
    let after = r#"
        CREATE NAMESPACE app;
        CREATE TABLE message_streams (
          id BIGINT PRIMARY KEY,
          body TEXT,
          priority BIGINT DEFAULT 2,
          created_at TIMESTAMP
        )
        WITH (COMPRESSION = 'zstd');
        CREATE TABLE app.new_table (id BIGINT PRIMARY KEY);
        CREATE TOPIC app.events WITH (retention_seconds = 7200);
        ALTER TOPIC app.events ADD SOURCE message_streams ON INSERT WHERE priority >= 10 WITH (payload = 'full');
        CREATE TOPIC app.new_topic PARTITIONS 2;
    "#;

    let diff = destructive_diff(before, after);

    assert_contains(
        &diff.up,
        "ALTER TABLE message_streams SET TBLPROPERTIES (COMPRESSION = 'zstd');",
    );
    assert_contains(&diff.up, "ALTER TABLE message_streams MODIFY COLUMN body TEXT NULL;");
    assert_contains(
        &diff.up,
        "ALTER TABLE message_streams MODIFY COLUMN priority BIGINT NULL DEFAULT 2;",
    );
    assert_contains(&diff.up, "ALTER TABLE message_streams ADD COLUMN created_at TIMESTAMP;");
    assert_contains(&diff.up, "ALTER TABLE message_streams DROP COLUMN obsolete;");
    assert_contains(&diff.up, "CREATE TABLE app.new_table");
    assert_contains(&diff.up, "DROP TABLE app.old_table;");
    assert_contains(
        &diff.up,
        "ALTER TOPIC app.events SET RETENTION WITH (retention_seconds = 7200);",
    );
    assert_contains(
        &diff.up,
        "ALTER TOPIC app.events ADD SOURCE message_streams ON INSERT WHERE priority >= 10 WITH (payload = 'full');",
    );
    assert_contains(&diff.up, "CREATE TOPIC app.new_topic PARTITIONS 2;");
    assert_contains(&diff.up, "DROP TOPIC app.old_topic;");
}

#[test]
fn run_multiple_times_diff() {
    let before = "CREATE TABLE app.users (id BIGINT PRIMARY KEY);";
    let after = "CREATE TABLE app.users (id BIGINT PRIMARY KEY, email TEXT NOT NULL);";

    let expected = diff(before, after).up;

    for _ in 0..20 {
        assert_eq!(diff(before, after).up, expected);
    }
}
