use std::fs;

use tempfile::TempDir;

use crate::test_project_workflow_dev::{
    create_isolated_cli_std_command, start_migration_tracking_server, store_test_dev_credentials,
    update_dev_project, write_owned_instance,
};

#[test]
fn test_project_workflow_pg18_schema_types_domains_routines_and_guardrails() {
    let (url, requests, _server_handle) = start_migration_tracking_server();
    let temp = TempDir::new().expect("temp dir");
    let isolated_home = temp.path().join("home");
    let credentials_path = temp.path().join("credentials.toml");
    let project_dir = temp.path().join("pg18-app");
    fs::create_dir_all(&project_dir).expect("create project dir");
    write_owned_instance(&project_dir, &url);
    store_test_dev_credentials(&credentials_path);

    // 1. Initialize disposable project
    let mut init_cmd = create_isolated_cli_std_command(&isolated_home, &credentials_path);
    init_cmd.current_dir(&project_dir).args([
        "init",
        "--yes",
        "--name",
        "pg18-app",
        "--schema-mode",
        "sql",
        "--languages",
        "typescript,dart",
    ]);
    let init_out = init_cmd.output().expect("init project");
    assert!(
        init_out.status.success(),
        "init failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&init_out.stdout),
        String::from_utf8_lossy(&init_out.stderr)
    );

    update_dev_project(&project_dir, |config| {
        config.dev.processes.clear();
        config.dev.watch = false;
        config.schema.watch = false;
        config.dev.apply_schema = true;
        config.dev.generate_types = true;
        config.connection.get_mut("dev").expect("dev env").url = url.clone();
    });

    // 2. Write PostgreSQL 18 schema with ENUM, composite type, DOMAIN, table, and routines
    let initial_schema = r#"
CREATE TYPE app.order_state AS ENUM ('pending', 'paid');
CREATE TYPE app.money_pair AS (currency TEXT, cents BIGINT);
CREATE DOMAIN app.positive_qty AS INTEGER NOT NULL CHECK (VALUE > 0);

CREATE TABLE app.orders (
    id BIGINT PRIMARY KEY,
    customer TEXT NOT NULL,
    qty app.positive_qty DEFAULT 1,
    state app.order_state NOT NULL DEFAULT 'pending',
    total app.money_pair
) WITH (TYPE = 'SHARED');

CREATE FUNCTION app.format_state(s app.order_state)
RETURNS TEXT
LANGUAGE javascript AS $fn$
-- UP inside dollar body must not be treated as a section marker
export default async function(_ctx, s) {
    return `state:${s}`;
}
$fn$;
"#;
    fs::write(project_dir.join("schema.sql"), initial_schema).expect("write initial schema.sql");

    // 3. Generate SDK types & procedure bindings from PG18 schema
    let mut gen_cmd = create_isolated_cli_std_command(&isolated_home, &credentials_path);
    gen_cmd.current_dir(&project_dir).args(["schema", "gen"]);
    let gen_out = gen_cmd.output().expect("schema gen");
    assert!(
        gen_out.status.success(),
        "schema gen failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&gen_out.stdout),
        String::from_utf8_lossy(&gen_out.stderr)
    );

    let ts_schema =
        fs::read_to_string(project_dir.join("src/generated/schema.ts")).expect("ts schema");
    assert!(ts_schema.contains("OrderState"), "expected OrderState enum in TS schema");
    assert!(ts_schema.contains("MoneyPair"), "expected MoneyPair struct in TS schema");
    assert!(
        ts_schema.contains("PositiveQty"),
        "expected PositiveQty domain alias in TS schema"
    );

    let dart_schema =
        fs::read_to_string(project_dir.join("lib/generated/kalam.dart")).expect("dart schema");
    assert!(
        dart_schema.contains("typedef AppPositiveQty = int"),
        "expected AppPositiveQty typedef in Dart schema:\n{dart_schema}"
    );

    // 4. Create initial migration and verify -- UP / -- DOWN marker handling with dollar-quoted
    //    body
    let mut mig_create = create_isolated_cli_std_command(&isolated_home, &credentials_path);
    mig_create
        .current_dir(&project_dir)
        .args(["db", "migration", "create", "init_pg18"]);
    let mig_out = mig_create.output().expect("migration create");
    assert!(
        mig_out.status.success(),
        "migration create failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&mig_out.stdout),
        String::from_utf8_lossy(&mig_out.stderr)
    );

    let mut migrate_cmd = create_isolated_cli_std_command(&isolated_home, &credentials_path);
    migrate_cmd.current_dir(&project_dir).args(["db", "migrate"]);
    let migrate_out = migrate_cmd.output().expect("db migrate");
    assert!(
        migrate_out.status.success(),
        "db migrate failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&migrate_out.stdout),
        String::from_utf8_lossy(&migrate_out.stderr)
    );

    // Establish baseline for evolved diff
    fs::write(project_dir.join("kalam/.schema-baseline.sql"), initial_schema)
        .expect("write baseline");

    // 5. Evolve schema: add enum value, composite attribute, and column; verify PG ALTER emission
    let evolved_schema = r#"
CREATE TYPE app.order_state AS ENUM ('pending', 'paid', 'shipped');
CREATE TYPE app.money_pair AS (currency TEXT, cents BIGINT, note TEXT);
CREATE DOMAIN app.positive_qty AS INTEGER NOT NULL CHECK (VALUE > 0);

CREATE TABLE app.orders (
    id BIGINT PRIMARY KEY,
    customer TEXT NOT NULL,
    qty app.positive_qty DEFAULT 1,
    state app.order_state NOT NULL DEFAULT 'pending',
    total app.money_pair,
    notes TEXT DEFAULT ''
) WITH (TYPE = 'SHARED');

CREATE FUNCTION app.format_state(s app.order_state)
RETURNS TEXT
LANGUAGE javascript AS $fn$
export default async function(_ctx, s) {
    return `state:${s}`;
}
$fn$;
"#;
    fs::write(project_dir.join("schema.sql"), evolved_schema).expect("write evolved schema.sql");

    let mut mig2_create = create_isolated_cli_std_command(&isolated_home, &credentials_path);
    mig2_create
        .current_dir(&project_dir)
        .args(["db", "migration", "create", "evolve_pg18"]);
    let mig2_out = mig2_create.output().expect("migration create evolve");
    assert!(
        mig2_out.status.success(),
        "migration create evolve failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&mig2_out.stdout),
        String::from_utf8_lossy(&mig2_out.stderr)
    );

    let migrations_dir = project_dir.join("kalam/migrations");
    let evolve_path = fs::read_dir(&migrations_dir)
        .expect("read migrations")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.contains("evolve_pg18") && name.ends_with(".sql"))
        })
        .expect("evolve_pg18 migration file");
    let evolve_sql = fs::read_to_string(evolve_path).expect("read evolve migration");
    assert!(
        evolve_sql.contains("ADD VALUE 'shipped'")
            && evolve_sql.contains("ADD ATTRIBUTE note")
            && evolve_sql.contains("ADD COLUMN notes"),
        "expected PG18 additive diff statements:\n{evolve_sql}"
    );
    assert!(!evolve_sql.contains("MODIFY COLUMN"));

    // 6. Invalid edit recovery: introduce syntax error, verify failure without erasing generated
    //    files, then fix
    fs::write(project_dir.join("schema.sql"), "CREATE TABLE app.broken (id );\n")
        .expect("write invalid schema");
    let mut bad_gen = create_isolated_cli_std_command(&isolated_home, &credentials_path);
    bad_gen.current_dir(&project_dir).args(["schema", "gen"]);
    let bad_out = bad_gen.output().expect("bad schema gen");
    assert!(!bad_out.status.success(), "invalid SQL must fail schema gen");
    assert!(
        project_dir.join("src/generated/schema.ts").exists(),
        "existing generated schema must remain intact on failed edit"
    );

    // Restore valid schema
    fs::write(project_dir.join("schema.sql"), evolved_schema).expect("restore valid schema");
    let mut good_gen = create_isolated_cli_std_command(&isolated_home, &credentials_path);
    good_gen.current_dir(&project_dir).args(["schema", "gen"]);
    assert!(good_gen.output().expect("good schema gen").status.success());

    let _ = requests;
}
