//! CALL + nested SQL/topic + EXECUTE ACL, plus schema-first type/procedure e2e.

use std::fs;

use serde_json::json;
use tempfile::TempDir;

use crate::{common::*, kobj_helpers::*};

fn create_js_procedure(ns: &str, name: &str, params: &str, body: &str) {
    exec(&format!(
        "CREATE OR REPLACE PROCEDURE {ns}.{name}({params}) LANGUAGE JAVASCRIPT AS $$\n{body}\n$$"
    ));
}

/// Matches `kalamdb_functions_host::namespace_object_ident` for typical namespace ids.
fn js_namespace_ident(namespace: &str) -> String {
    let mut pascal = String::new();
    let mut capitalize = true;
    for ch in namespace.chars() {
        if ch == '_' || ch == '-' || ch == '.' {
            capitalize = true;
            continue;
        }
        if capitalize {
            pascal.extend(ch.to_uppercase());
            capitalize = false;
        } else {
            pascal.push(ch);
        }
    }
    let mut chars = pascal.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
        None => "value".to_string(),
    }
}

fn rest_post(path: &str, body: serde_json::Value) -> (u16, String) {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let token = get_access_token(default_username(), default_password())
            .await
            .expect("root token");
        let response = shared_http_client()
            .post(format!("{}{path}", server_url()))
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .expect("HTTP POST");
        let status = response.status().as_u16();
        let text = response.text().await.expect("HTTP body");
        (status, text)
    })
}

fn run_kalam(project_dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    let mut cmd = create_cli_command_with_root_auth();
    clear_workflow_url_env_overrides_assert_cmd(&mut cmd);
    cmd.current_dir(project_dir).args(args);
    let output = cmd.output().unwrap_or_else(|err| panic!("kalam {args:?}: {err}"));
    assert!(
        output.status.success(),
        "kalam {args:?} failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn procedure_source_uses_abi_v2_db(orders: &str, topic: &str) -> String {
    format!(
        "return ctx.db.execute(\"INSERT INTO {orders} (id, status) VALUES (\" + input.p_id + \", \
         'ok')\").then(function () {{\n  return ctx.topics.publish('{topic}', {{ id: input.p_id, \
         status: 'ok' }});\n}}).then(function () {{\n  return {{ id: input.p_id, status: \
         'ok' }};\n}});"
    )
}

#[ntest::timeout(300000)]
#[test]
fn kobj_functions_checkpoint_c_call_nested_db_and_topic() {
    if skip_if_no_server() {
        return;
    }
    let ns = setup_namespace("kobj_fn");
    let orders = format!("{ns}.fn_orders");
    exec(&format!(
        "CREATE TABLE {orders} (id INT PRIMARY KEY, status TEXT) WITH (TYPE = 'SHARED')"
    ));
    grant_public_shared_table_access(&orders);
    ready(&orders);
    let topic = format!("{ns}.fn_events");
    exec(&format!("CREATE TOPIC {topic}"));

    create_js_procedure(&ns, "echo", "msg TEXT", "return input.msg;");
    create_js_procedure(&ns, "inc", "x INT", "return input.x + 1;");
    create_js_procedure(
        &ns,
        "plus_one",
        "x INT",
        &format!("return ctx.functions.call('{ns}.inc', [input]);"),
    );
    create_js_procedure(&ns, "boom", "", "throw new Error('boom');");
    create_js_procedure(&ns, "wrap_boom", "", &format!("return ctx.functions.call('{ns}.boom');"));
    create_js_procedure(
        &ns,
        "place_order",
        "p_id INT",
        &procedure_source_uses_abi_v2_db(&orders, &topic),
    );

    let echo_rows = query_rows(&format!("CALL {ns}.echo('hello')"));
    assert_eq!(echo_rows.len(), 1, "CALL echo should return one row: {echo_rows:?}");
    let echoed = cell(&echo_rows[0], "result");
    assert!(
        echoed.as_str() == Some("hello") || echoed.to_string().contains("hello"),
        "echoed value: {echoed}"
    );

    let nested = query_rows(&format!("CALL {ns}.plus_one(41)"));
    let nested_value = cell_i64(&nested[0], "result")
        .unwrap_or_else(|| cell(&nested[0], "result").as_i64().unwrap_or(-1));
    assert_eq!(nested_value, 42, "nested CALL should return 42: {nested:?}");

    let inc = query_rows(&format!("CALL {ns}.inc(41)"));
    let inc_value = cell_i64(&inc[0], "result")
        .unwrap_or_else(|| cell(&inc[0], "result").as_i64().unwrap_or(-1));
    assert_eq!(inc_value, 42, "CALL inc(41) should return 42: {inc:?}");

    let boom = exec_err(&format!("CALL {ns}.wrap_boom()"));
    let boom_lower = boom.to_ascii_lowercase();
    assert!(
        boom_lower.contains("wrap_boom") && boom_lower.contains("boom"),
        "nested error must include the call stack: {boom}"
    );

    exec(&format!("CALL {ns}.place_order(7)"));
    assert_eq!(count_sql(&format!("SELECT COUNT(*) FROM {orders} WHERE id = 7")), 1);
    let consumed = query_rows(&format!("CONSUME FROM {topic} FROM EARLIEST LIMIT 10"));
    assert!(
        !consumed.is_empty(),
        "typed topic publish should be visible after CALL commit: {consumed:?}"
    );

    exec(&format!("BEGIN; CALL {ns}.place_order(99); ROLLBACK;"));
    assert_eq!(
        count_sql(&format!("SELECT COUNT(*) FROM {orders} WHERE id = 99")),
        0,
        "rollback must drop nested INSERT"
    );
    let after_rollback = query_rows(&format!("CONSUME FROM {topic} FROM EARLIEST LIMIT 10"));
    assert_eq!(
        after_rollback.len(),
        consumed.len(),
        "rollback must drop staged topic publish: before={consumed:?} after={after_rollback:?}"
    );

    let (username, password) = create_login_user("fnuser");
    let denied =
        execute_sql_via_client_as(&username, &password, &format!("CALL {ns}.echo('nope')"));
    assert!(denied.is_err(), "user without EXECUTE must be denied");

    exec(&format!("GRANT EXECUTE ON PROCEDURE {ns}.echo TO user"));
    execute_sql_via_client_as(&username, &password, &format!("CALL {ns}.echo('ok')"))
        .unwrap_or_else(|err| panic!("granted user CALL should succeed: {err}"));
    exec(&format!("REVOKE EXECUTE ON PROCEDURE {ns}.echo FROM user"));
    let revoked =
        execute_sql_via_client_as(&username, &password, &format!("CALL {ns}.echo('later')"));
    assert!(revoked.is_err(), "revoked EXECUTE must deny CALL");

    let (status, body) = rest_post(&format!("/v1/functions/{ns}/echo"), json!({ "msg": "rest" }));
    assert!(
        (200..300).contains(&status),
        "REST /v1/functions should succeed: {status} {body}"
    );
    assert!(
        body.to_ascii_lowercase().contains("rest"),
        "REST result should echo the argument: {body}"
    );
}

#[ntest::timeout(300000)]
#[test]
fn kobj_functions_topic_trigger_delivers_and_acks() {
    if skip_if_no_server() {
        return;
    }
    let ns = setup_namespace("kobj_trig");
    let hits = format!("{ns}.trig_hits");
    exec(&format!(
        "CREATE TABLE {hits} (id INT PRIMARY KEY, note TEXT) WITH (TYPE = 'SHARED')"
    ));
    grant_public_shared_table_access(&hits);
    ready(&hits);
    let topic = format!("{ns}.trig_events");
    exec(&format!("CREATE TOPIC {topic}"));
    create_js_procedure(
        &ns,
        "on_trig",
        "payload TEXT",
        &format!(
            "var payload = input && input.payload != null ? input.payload : input;\nif (typeof \
             payload === 'string') {{ try {{ payload = JSON.parse(payload); }} catch (e) {{}} \
             }}\nvar id = (payload && payload.id != null) ? payload.id : payload;\nreturn \
             ctx.db.execute(\"INSERT INTO {hits} (id, note) VALUES (\" + id + \", \
             'ok')\").then(function () {{ return payload; }});"
        ),
    );
    create_js_procedure(
        &ns,
        "publish_trig",
        "p_id INT",
        &format!(
            "return ctx.topics.publish('{topic}', {{ id: input.p_id }}).then(function () {{ \
             return input.p_id; }});"
        ),
    );
    exec(&format!(
        "CREATE TRIGGER {ns}.on_trig_event ON TOPIC {topic} EXECUTE PROCEDURE \
         {ns}.on_trig(PAYLOAD) WITH (start = 'latest', retries = 3, retry_backoff = '100ms')"
    ));
    exec(&format!("CALL {ns}.publish_trig(7)"));

    let mut delivered = false;
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(250));
        if count_sql(&format!("SELECT COUNT(*) FROM {hits} WHERE id = 7")) == 1 {
            delivered = true;
            break;
        }
    }
    assert!(delivered, "trigger should insert into {hits} after topic publish");
    let attempts = query_rows(&format!(
        "SELECT status FROM system.trigger_attempts WHERE event_id LIKE '%{topic}%'"
    ));
    assert!(
        attempts.iter().any(|row| cell(row, "status").as_str() == Some("succeeded")),
        "trigger attempt should be succeeded: {attempts:?}"
    );
}

#[ntest::timeout(120000)]
#[test]
fn kobj_functions_active_runs_and_structured_errors() {
    if skip_if_no_server() {
        return;
    }
    let ns = setup_namespace("fnobs");
    create_js_procedure(&ns, "ok", "", "return 1;");
    create_js_procedure(&ns, "boom", "", "throw new Error('boom');");
    let _ = query_rows(&format!("CALL {ns}.ok()"));
    let runs = query_rows("SELECT execution_id FROM system.active_function_runs");
    assert!(
        runs.is_empty(),
        "finished roots must leave system.active_function_runs: {runs:?}"
    );
    let boom = exec_err(&format!("CALL {ns}.boom()"));
    assert!(boom.to_ascii_lowercase().contains("boom"), "boom error: {boom}");
    let errors = query_rows(&format!(
        "SELECT code, routine_id FROM system.function_errors WHERE routine_id LIKE '%{ns}.boom%'"
    ));
    assert!(
        !errors.is_empty(),
        "structured function errors should appear in system.function_errors: {errors:?}"
    );
}

#[ntest::timeout(180000)]
#[test]
fn kobj_functions_create_type_and_inline_procedure() {
    if skip_if_no_server() {
        return;
    }
    let ns = setup_namespace("kobj_ftype");
    exec(&format!("CREATE TYPE {ns}.address AS (city TEXT, country TEXT)"));
    exec(&format!("CREATE TYPE {ns}.status AS ENUM ('active', 'blocked')"));

    let types =
        query_rows(&format!("SELECT type_id, kind FROM system.types WHERE type_id LIKE '{ns}.%'"));
    let kinds: Vec<String> = types.iter().filter_map(|row| cell_str(row, "kind")).collect();
    assert!(
        kinds.iter().any(|kind| kind.contains("composite")),
        "expected composite type in system.types: {types:?}"
    );
    assert!(
        kinds.iter().any(|kind| kind.contains("enum")),
        "expected enum type in system.types: {types:?}"
    );

    create_js_procedure(
        &ns,
        "echo_addr",
        &format!("addr {ns}.address"),
        "ctx.log.info('echo_addr', { city: input && input.addr && input.addr.city });\nreturn input.addr;",
    );
    create_js_procedure(&ns, "echo_status", &format!("s {ns}.status"), "return String(input.s);");
    create_js_procedure(
        &ns,
        "health",
        "",
        "return ctx.db.query('SELECT 1 AS n').then(function () { return 'ok'; });",
    );

    let health = query_rows(&format!("CALL {ns}.health()"));
    let health_value = cell(&health[0], "result");
    assert!(
        health_value.as_str() == Some("ok") || health_value.to_string().contains("ok"),
        "inline health should return ok: {health:?}"
    );

    let status_rows = query_rows(&format!("CALL {ns}.echo_status('active')"));
    let status_value = cell(&status_rows[0], "result");
    assert!(
        status_value.as_str() == Some("active") || status_value.to_string().contains("active"),
        "enum CALL should round-trip: {status_rows:?}"
    );

    let (http_status, body) = rest_post(
        &format!("/v1/functions/{ns}/echo_addr"),
        json!({ "addr": { "city": "Paris", "country": "FR" } }),
    );
    assert!(
        (200..300).contains(&http_status),
        "REST composite CALL should succeed: {http_status} {body}"
    );
    assert!(
        body.contains("Paris") && body.contains("FR"),
        "REST composite result should round-trip fields: {body}"
    );
}

#[ntest::timeout(180000)]
#[test]
fn kobj_functions_typed_nested_call_and_log() {
    if skip_if_no_server() {
        return;
    }
    let ns = setup_namespace("kobj_typed");
    let ident = js_namespace_ident(&ns);
    create_js_procedure(&ns, "inc", "x INT", "return input.x + 1;");
    create_js_procedure(
        &ns,
        "plus_one",
        "x INT",
        &format!("return ctx.functions.{ident}.inc(input);"),
    );
    create_js_procedure(
        &ns,
        "logged",
        "x INT",
        r#"
ctx.log.debug('debug', { n: input.x });
ctx.log.info('hello', { n: input.x });
ctx.log.warn('warn');
console.log('console', { n: input.x });
try { throw new Error('sample'); } catch (e) { ctx.log.error(e, 'failed', { id: input.x }); }
return input.x;
"#,
    );

    let nested = query_rows(&format!("CALL {ns}.plus_one(41)"));
    let nested_value = cell_i64(&nested[0], "result")
        .unwrap_or_else(|| cell(&nested[0], "result").as_i64().unwrap_or(-1));
    assert_eq!(
        nested_value, 42,
        "typed nested CALL ctx.functions.{ident}.inc should return 42: {nested:?}"
    );

    let logged = query_rows(&format!("CALL {ns}.logged(7)"));
    let logged_value = cell_i64(&logged[0], "result")
        .unwrap_or_else(|| cell(&logged[0], "result").as_i64().unwrap_or(-1));
    assert_eq!(logged_value, 7, "ctx.log must not break CALL: {logged:?}");
}

#[ntest::timeout(180000)]
#[test]
fn kobj_functions_create_validates_javascript_and_reports_replace() {
    if skip_if_no_server() {
        return;
    }
    let ns = setup_namespace("kobj_fnlint");
    let log_as_fn = exec_err(&format!(
        "CREATE PROCEDURE {ns}.health() RETURNS TEXT LANGUAGE JAVASCRIPT AS $$\n  \
         ctx.log('ttt');\n  return 'ok';\n$$"
    ));
    assert!(
        log_as_fn.contains("ctx.log is not a function"),
        "CREATE must reject ctx.log(...): {log_as_fn}"
    );

    let created = exec(&format!(
        "CREATE PROCEDURE {ns}.health() RETURNS TEXT LANGUAGE JAVASCRIPT AS $$\n  \
         console.log('ok');\n  ctx.log.info('ok');\n  return 'ok';\n$$"
    ));
    assert!(
        created.contains(&format!("Procedure {ns}.health created")),
        "first CREATE should report created: {created}"
    );
    assert!(
        created.contains("implementation:"),
        "CREATE should report implementation: {created}"
    );
    assert!(
        !created.contains("module_revision:") || created.contains("not created by this statement"),
        "inline CREATE must not claim a new module revision: {created}"
    );
    let health = query_rows(&format!("CALL {ns}.health()"));
    let health_value = cell(&health[0], "result");
    assert!(
        health_value.as_str() == Some("ok") || health_value.to_string().contains("ok"),
        "console.log plus ctx.log.info must not break CALL: {health:?}"
    );

    let replaced = exec(&format!(
        "CREATE OR REPLACE PROCEDURE {ns}.health() RETURNS TEXT LANGUAGE JAVASCRIPT AS $$\n  \
         ctx.log.info('ok2');\n  return 'ok2';\n$$"
    ));
    assert!(replaced.contains("replaced"), "OR REPLACE should report replaced: {replaced}");
    assert!(
        replaced.contains("source: changed"),
        "changed body should report source: changed: {replaced}"
    );
}

#[ntest::timeout(300000)]
#[test]
fn kobj_functions_project_backed_create_type_and_procedure() {
    if skip_if_no_server() {
        return;
    }
    let ns = setup_namespace("kobj_fncode");
    let ident = js_namespace_ident(&ns);
    let schema_sql = format!(
        "CREATE TYPE {ns}.address AS (city TEXT, country TEXT);\nCREATE TYPE {ns}.status AS ENUM \
         ('active', 'blocked');\nCREATE PROCEDURE {ns}.health() RETURNS TEXT;\nCREATE PROCEDURE \
         {ns}.inc(x INT) RETURNS INT;\nCREATE PROCEDURE {ns}.plus_one(x INT) RETURNS INT;\n"
    );
    exec(&schema_sql);

    let missing = exec_err(&format!("CALL {ns}.health()"));
    let missing_lower = missing.to_ascii_lowercase();
    assert!(
        missing_lower.contains("not implemented")
            || missing_lower.contains("procedure_not_implemented")
            || missing_lower.contains("missing export"),
        "bodyless procedure must fail until this test activates its module: {missing}"
    );

    let temp = TempDir::new().expect("temp dir");
    let project_dir = temp.path().join("fn-app");
    fs::create_dir_all(&project_dir).expect("create project dir");

    let mut init = create_cli_command();
    clear_workflow_url_env_overrides_assert_cmd(&mut init);
    init.current_dir(&project_dir).args([
        "init",
        "--yes",
        "--name",
        "fn-app",
        "--schema-mode",
        "sql",
        "--languages",
        "typescript",
        "--server-mode",
        "remote",
        "--server-url",
        server_url(),
    ]);
    let init_out = init.output().expect("init");
    assert!(
        init_out.status.success(),
        "init failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&init_out.stdout),
        String::from_utf8_lossy(&init_out.stderr)
    );

    login_kalam_dev_for_project(&project_dir);
    run_kalam(
        &project_dir,
        &[
            "link",
            "--env",
            "dev",
            "--url",
            server_url(),
            "--namespace",
            &ns,
        ],
    );
    fs::write(project_dir.join("schema.sql"), &schema_sql).expect("write schema.sql");
    run_kalam(&project_dir, &["schema", "gen"]);

    let runtime_dts =
        fs::read_to_string(project_dir.join("functions/src/generated/runtime.d.ts"))
            .expect("runtime.d.ts");
    assert!(
        runtime_dts.contains(&format!("{ident}:")),
        "generated FunctionsHost should nest schema {ident}: {runtime_dts}"
    );
    assert!(
        runtime_dts.contains("plusOne(") || runtime_dts.contains("plus_one"),
        "generated runtime should mention plus_one: {runtime_dts}"
    );

    let src_dir = project_dir.join("functions/src").join(&ns);
    fs::create_dir_all(&src_dir).expect("functions src");
    fs::write(
        src_dir.join("health.ts"),
        "export default async (ctx, input) => {\n  ctx.log.info('project health');\n  return \
         'ok';\n};\n",
    )
    .expect("health.ts");
    fs::write(src_dir.join("inc.ts"), "export default async (ctx, input) => input.x + 1;\n")
        .expect("inc.ts");
    fs::write(
        src_dir.join("plus_one.ts"),
        format!("export default async (ctx, input) => ctx.functions.{ident}.inc(input);\n"),
    )
    .expect("plus_one.ts");

    run_kalam(&project_dir, &["functions", "build"]);

    let artifact = fs::read_to_string(project_dir.join("functions/.kalam/build/module.js"))
        .expect("module.js");
    let manifest: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(project_dir.join("functions/.kalam/build/manifest.json"))
            .expect("manifest.json"),
    )
    .expect("parse manifest");
    let module = manifest.get("module").and_then(|v| v.as_str()).unwrap_or("backend");
    let contract_hash = manifest.get("contractHash").and_then(|v| v.as_str()).unwrap_or_default();
    let abi_version = manifest.get("abiVersion").and_then(|v| v.as_u64()).unwrap_or(2);
    let mut exports = Vec::new();
    if let Some(serde_json::Value::Object(procedures)) = manifest.get("procedures") {
        for (name, kind) in procedures {
            if kind.as_str() == Some("module") {
                exports.push(name.clone());
            }
        }
    }

    let (status, body) = rest_post(
        &format!("/v1/api/functions/modules/{module}/activate"),
        json!({
            "artifact": artifact,
            "contractHash": contract_hash,
            "abiVersion": abi_version,
            "exports": exports,
        }),
    );
    assert!((200..300).contains(&status), "activate should succeed: {status} {body}");

    let health = query_rows(&format!("CALL {ns}.health()"));
    let health_value = cell(&health[0], "result");
    assert!(
        health_value.as_str() == Some("ok") || health_value.to_string().contains("ok"),
        "project-backed health should return ok after activate: {health:?}"
    );

    let nested = query_rows(&format!("CALL {ns}.plus_one(41)"));
    let nested_value = cell_i64(&nested[0], "result")
        .unwrap_or_else(|| cell(&nested[0], "result").as_i64().unwrap_or(-1));
    assert_eq!(
        nested_value, 42,
        "project-backed typed nested CALL should return 42: {nested:?}"
    );
}

#[ntest::timeout(180000)]
#[test]
fn kobj_functions_stream_insert_is_visible_inside_procedure() {
    if skip_if_no_server() {
        return;
    }
    let ns = setup_namespace("kobj_fnstream");
    let stream = format!("{ns}.fn_stream");
    exec(&format!(
        "CREATE STREAM TABLE {stream} (id INT PRIMARY KEY, note TEXT) WITH (TTL_SECONDS = 30)"
    ));
    ready(&stream);
    create_js_procedure(
        &ns,
        "write_stream",
        "p_id INT",
        &format!(
            "return ctx.db.execute(\"INSERT INTO {stream} (id, note) VALUES (\" + input.p_id + \
             \", 'ok')\").then(function () {{\n  return ctx.db.query(\"SELECT note FROM {stream} \
             WHERE id = \" + input.p_id);\n}});"
        ),
    );
    let rows = query_rows(&format!("CALL {ns}.write_stream(9)"));
    let value = cell(&rows[0], "result");
    assert!(
        value.to_string().contains("ok"),
        "STREAM INSERT inside a procedure should be readable before return: {rows:?}"
    );
}

#[ntest::timeout(120000)]
#[test]
fn kobj_functions_sleep_then_returns() {
    if skip_if_no_server() {
        return;
    }
    let ns = setup_namespace("kobj_fnsleep");
    create_js_procedure(
        &ns,
        "nap",
        "ms INT",
        "return ctx.sleep(input.ms).then(function () { return 'awake'; });",
    );
    let started = std::time::Instant::now();
    let rows = query_rows(&format!("CALL {ns}.nap(50)"));
    let elapsed_ms = started.elapsed().as_millis();
    let value = cell(&rows[0], "result");
    assert!(
        value.as_str() == Some("awake") || value.to_string().contains("awake"),
        "ctx.sleep should resolve and return: {rows:?}"
    );
    assert!(
        elapsed_ms >= 40,
        "ctx.sleep(50) should pause the isolate: {elapsed_ms}ms"
    );
}

#[ntest::timeout(120000)]
#[test]
fn kobj_functions_named_multi_arg_call_input() {
    if skip_if_no_server() {
        return;
    }
    let ns = setup_namespace("kobj_fngreet");
    create_js_procedure(
        &ns,
        "greet",
        "first TEXT, last TEXT",
        "return input.first + ' ' + input.last;",
    );
    let rows = query_rows(&format!("CALL {ns}.greet('Ada', 'Lovelace')"));
    let value = cell(&rows[0], "result");
    assert!(
        value.as_str() == Some("Ada Lovelace") || value.to_string().contains("Ada Lovelace"),
        "two CALL args should pack into a named object: {rows:?}"
    );
}
