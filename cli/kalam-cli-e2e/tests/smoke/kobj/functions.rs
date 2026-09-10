//! CALL + nested SQL/topic + EXECUTE ACL, plus schema-first type/procedure e2e.

use std::{collections::HashMap, fs, path::Path};

use serde_json::{json, Value};
use tempfile::TempDir;

use crate::{common::*, kobj_helpers::*};

fn create_js_procedure(ns: &str, name: &str, params: &str, body: &str) {
    exec(&format!(
        "CREATE OR REPLACE PROCEDURE {ns}.{name}({params}) LANGUAGE JAVASCRIPT AS $$\n{body}\n$$"
    ));
}

/// Matches `kalamdb_functions_host::camel_case` for SQL identifiers used from JS.
fn js_camel_ident(value: &str) -> String {
    let mut pascal = String::new();
    let mut capitalize = true;
    for ch in value.chars() {
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

/// Matches `kalamdb_functions_host::namespace_object_ident` for typical namespace ids.
fn js_namespace_ident(namespace: &str) -> String {
    js_camel_ident(namespace)
}

/// Matches `kalamdb_functions_host::method_ident` (`child_log` → `childLog`).
fn js_method_ident(name: &str) -> String {
    js_camel_ident(name)
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

fn link_dir(src: &Path, dest: &Path) {
    if dest.exists() {
        return;
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).expect("node_modules parent");
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(src, dest)
        .unwrap_or_else(|err| panic!("symlink {} -> {}: {err}", src.display(), dest.display()));
    #[cfg(not(unix))]
    panic!("function build smoke requires unix symlinks for node_modules");
}

fn link_function_build_deps(project_dir: &Path) {
    let example_modules = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/chat-with-ai/node_modules");
    assert!(
        example_modules.join("esbuild/bin/esbuild").is_file(),
        "examples/chat-with-ai/node_modules/esbuild is required for functions build smoke"
    );
    let dest = project_dir.join("node_modules");
    for name in ["esbuild", "drizzle-orm"] {
        link_dir(&example_modules.join(name), &dest.join(name));
    }
    for scoped in ["@esbuild", "@kalamdb"] {
        let src_scope = example_modules.join(scoped);
        if !src_scope.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&src_scope).expect("scoped packages") {
            let entry = entry.expect("scoped package entry");
            link_dir(&entry.path(), &dest.join(scoped).join(entry.file_name()));
        }
    }
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
         status: 'ok' }});\n}}).then(function () {{\n  return {{ id: input.p_id, status: 'ok' \
         }};\n}});"
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
    let runs = query_rows("SELECT execution_id FROM system.active_procedure_runs");
    assert!(
        runs.is_empty(),
        "finished roots must leave system.active_procedure_runs: {runs:?}"
    );
    let stats = query_rows(
        "SELECT metric_name, metric_value FROM system.stats WHERE metric_name LIKE \
         'function_memory%' OR metric_name LIKE 'function_instances%'",
    );
    assert!(
        stats
            .iter()
            .any(|row| cell_str(row, "metric_name").as_deref()
                == Some("function_memory_limit_bytes")),
        "system.stats should expose function memory budgets: {stats:?}"
    );
    let instances = query_rows(
        "SELECT instance_id, worker, state, reserved_bytes FROM system.module_instances",
    );
    assert!(
        instances.iter().any(|row| cell_str(row, "state").as_deref() == Some("idle")
            || cell_str(row, "state").as_deref() == Some("active")),
        "successful CALL should leave a resident isolate in system.module_instances: {instances:?}"
    );
    let procedures = query_rows(&format!(
        "SELECT procedure_id, implementation, signature FROM system.procedures WHERE procedure_id \
         LIKE '%{ns}.ok%'"
    ));
    assert!(
        procedures
            .iter()
            .any(|row| cell_str(row, "implementation").as_deref() == Some("inline")),
        "system.procedures should list the inline CALL: {procedures:?}"
    );
    let boom = exec_err(&format!("CALL {ns}.boom()"));
    assert!(boom.to_ascii_lowercase().contains("boom"), "boom error: {boom}");
    let errors = query_rows(&format!(
        "SELECT error_code, procedure_id FROM system.procedure_logs WHERE procedure_id LIKE \
         '%{ns}.boom%' AND outcome = 'error'"
    ));
    assert!(
        !errors.is_empty(),
        "structured procedure errors should appear in system.procedure_logs: {errors:?}"
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
        "ctx.log.info('echo_addr', { city: input && input.addr && input.addr.city });\nreturn \
         input.addr;",
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

fn procedure_logs_like(procedure_id: &str) -> Vec<HashMap<String, Value>> {
    query_rows(&format!(
        "SELECT procedure_id, outcome, channel, level, error_code, message FROM \
         system.procedure_logs WHERE procedure_id LIKE '%{procedure_id}%'"
    ))
}

fn log_messages_contain(rows: &[HashMap<String, Value>], needle: &str) -> bool {
    rows.iter()
        .any(|row| cell_str(row, "message").is_some_and(|message| message.contains(needle)))
}

fn log_row_matches(
    rows: &[HashMap<String, Value>],
    outcome: &str,
    channel: Option<&str>,
    needle: &str,
) -> bool {
    rows.iter().any(|row| {
        cell_str(row, "outcome").as_deref() == Some(outcome)
            && channel
                .map(|expected| cell_str(row, "channel").as_deref() == Some(expected))
                .unwrap_or(true)
            && cell_str(row, "message").is_some_and(|message| message.contains(needle))
    })
}

#[ntest::timeout(180000)]
#[test]
fn kobj_functions_v8_output_in_procedure_logs() {
    if skip_if_no_server() {
        return;
    }
    let ns = setup_namespace("kobj_v8log");
    let ident = js_namespace_ident(&ns);
    let console_marker = format!("v8-console-{ns}");
    let ctx_marker = format!("v8-ctxlog-{ns}");
    let caught_marker = format!("v8-caught-{ns}");
    let throw_marker = format!("v8-throw-{ns}");
    let reject_marker = format!("v8-reject-{ns}");
    let child_marker = format!("v8-child-{ns}");

    create_js_procedure(
        &ns,
        "logged",
        "",
        &format!(
            "console.log('{console_marker}');\nctx.log.info('{ctx_marker}');\ntry {{ throw new \
             Error('{caught_marker}'); }} catch (e) {{ ctx.log.error(e); }}\nreturn 1;"
        ),
    );
    create_js_procedure(&ns, "boom", "", &format!("throw new Error('{throw_marker}');"));
    create_js_procedure(
        &ns,
        "log_then_throw",
        "",
        &format!(
            "console.log('{console_marker}-then-throw');\nthrow new \
             Error('{throw_marker}-after-log');"
        ),
    );
    create_js_procedure(
        &ns,
        "reject",
        "",
        &format!("Promise.reject(new Error('{reject_marker}')); return 1;"),
    );
    create_js_procedure(&ns, "child_log", "", &format!("console.log('{child_marker}'); return 1;"));
    create_js_procedure(
        &ns,
        "parent_log",
        "",
        &format!("return ctx.functions.{ident}.{}();", js_method_ident("child_log")),
    );

    let logged = query_rows(&format!("CALL {ns}.logged()"));
    let logged_value = cell_i64(&logged[0], "result")
        .unwrap_or_else(|| cell(&logged[0], "result").as_i64().unwrap_or(-1));
    assert_eq!(logged_value, 1, "logged CALL should succeed: {logged:?}");

    let logged_rows = procedure_logs_like(&format!("{ns}.logged"));
    assert!(
        log_row_matches(&logged_rows, "log", Some("console"), &console_marker),
        "console.log must appear in system.procedure_logs: {logged_rows:?}"
    );
    assert!(
        log_row_matches(&logged_rows, "log", Some("ctx.log"), &ctx_marker),
        "ctx.log.info must appear in system.procedure_logs: {logged_rows:?}"
    );
    assert!(
        log_row_matches(&logged_rows, "log", Some("ctx.log"), &caught_marker),
        "caught JS exceptions logged via ctx.log.error must appear: {logged_rows:?}"
    );
    assert!(
        logged_rows.iter().any(|row| cell_str(row, "outcome").as_deref() == Some("ok")),
        "successful CALL should write outcome=ok: {logged_rows:?}"
    );

    let boom = exec_err(&format!("CALL {ns}.boom()"));
    assert!(boom.contains(&throw_marker), "uncaught throw must surface the V8 error: {boom}");
    let boom_rows = procedure_logs_like(&format!("{ns}.boom"));
    assert!(
        log_row_matches(&boom_rows, "error", Some("invocation"), &throw_marker),
        "uncaught throw must be in system.procedure_logs: {boom_rows:?}"
    );

    let both = exec_err(&format!("CALL {ns}.log_then_throw()"));
    assert!(
        both.contains(&format!("{throw_marker}-after-log")),
        "log-then-throw must still fail: {both}"
    );
    let both_rows = procedure_logs_like(&format!("{ns}.log_then_throw"));
    assert!(
        log_row_matches(
            &both_rows,
            "log",
            Some("console"),
            &format!("{console_marker}-then-throw")
        ),
        "console.log before throw must be persisted: {both_rows:?}"
    );
    assert!(
        log_row_matches(
            &both_rows,
            "error",
            Some("invocation"),
            &format!("{throw_marker}-after-log")
        ),
        "throw after console.log must be persisted: {both_rows:?}"
    );

    let rejected = exec_err(&format!("CALL {ns}.reject()"));
    assert!(
        rejected.contains(&reject_marker),
        "unhandled rejection must surface the JS reason: {rejected}"
    );
    let reject_rows = procedure_logs_like(&format!("{ns}.reject"));
    assert!(
        log_messages_contain(&reject_rows, &reject_marker),
        "unhandled rejection must be in system.procedure_logs: {reject_rows:?}"
    );

    let nested = query_rows(&format!("CALL {ns}.parent_log()"));
    let nested_value = cell_i64(&nested[0], "result")
        .unwrap_or_else(|| cell(&nested[0], "result").as_i64().unwrap_or(-1));
    assert_eq!(nested_value, 1, "nested CALL should succeed: {nested:?}");
    let child_rows = procedure_logs_like(&format!("{ns}.child_log"));
    assert!(
        log_row_matches(&child_rows, "log", Some("console"), &child_marker),
        "nested procedure console.log must be attributed to the child: {child_rows:?}"
    );
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
         {ns}.inc(x INT) RETURNS INT;\nCREATE PROCEDURE {ns}.plus_one(x INT) RETURNS INT;\nCREATE \
         PROCEDURE {ns}.greet() RETURNS TEXT LANGUAGE JAVASCRIPT AS $$ return 'inline'; $$;\n"
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

    let runtime_dts = fs::read_to_string(project_dir.join("functions/src/generated/runtime.d.ts"))
        .expect("runtime.d.ts");
    assert!(
        runtime_dts.contains(&format!("{ident}:")),
        "generated FunctionsHost should nest schema {ident}: {runtime_dts}"
    );
    assert!(
        runtime_dts.contains("plusOne(") || runtime_dts.contains("plus_one"),
        "generated runtime should mention plus_one: {runtime_dts}"
    );
    let procedure_dts =
        fs::read_to_string(project_dir.join("functions/src/generated/procedure.d.ts"))
            .expect("procedure.d.ts");
    assert!(
        procedure_dts.contains("ProcedureBuilder"),
        "generated procedure builders should exist: {procedure_dts}"
    );
    assert!(!project_dir.join("functions/src/generated/procedure.ts").exists());

    let src_dir = project_dir.join("functions/src").join(&ns);
    fs::create_dir_all(&src_dir).expect("functions src");
    let unimplemented = fs::read_to_string(src_dir.join("plus_one.ts")).expect("plus_one scaffold");
    assert!(
        unimplemented.contains(".unimplemented()"),
        "bodyless scaffold should be unimplemented: {unimplemented}"
    );
    fs::write(
        src_dir.join("handlers.ts"),
        format!(
            "import {{ procedure }} from \"../generated/contracts\";\nexport const health = \
             procedure.{ident}.health(async (ctx) => {{\n  ctx.log.info('project health');\n  \
             return 'ok';\n}});\nexport const inc = procedure.{ident}.inc(async (_ctx, input) => \
             input.x + 1);\n"
        ),
    )
    .expect("handlers.ts");
    fs::remove_file(src_dir.join("health.ts")).ok();
    fs::remove_file(src_dir.join("inc.ts")).ok();

    link_function_build_deps(&project_dir);
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

    let inc = query_rows(&format!("CALL {ns}.inc(41)"));
    let inc_value = cell_i64(&inc[0], "result")
        .unwrap_or_else(|| cell(&inc[0], "result").as_i64().unwrap_or(-1));
    assert_eq!(inc_value, 42, "two named handlers in one file should dispatch: {inc:?}");

    let unimplemented = exec_err(&format!("CALL {ns}.plus_one(41)"));
    let unimplemented_lower = unimplemented.to_ascii_lowercase();
    assert!(
        unimplemented_lower.contains("not implemented")
            || unimplemented_lower.contains("procedure_not_implemented"),
        "unimplemented bodyless routine must stay PROCEDURE_NOT_IMPLEMENTED: {unimplemented}"
    );

    let greet = query_rows(&format!("CALL {ns}.greet()"));
    let greet_value = cell(&greet[0], "result");
    assert!(
        greet_value.as_str() == Some("inline") || greet_value.to_string().contains("inline"),
        "inline body should run until a named override exists: {greet:?}"
    );

    fs::write(
        src_dir.join("plus_one.ts"),
        format!(
            "import {{ procedure }} from \"../generated/contracts\";\nexport const plusOne = \
             procedure.{ident}.plusOne(async (ctx, input) => ctx.functions.{ident}.inc(input));\n"
        ),
    )
    .expect("plus_one.ts");
    fs::write(
        src_dir.join("greet.ts"),
        format!(
            "import {{ procedure }} from \"../generated/contracts\";\nexport const greet = \
             procedure.{ident}.greet(async () => 'project');\n"
        ),
    )
    .expect("greet.ts");
    run_kalam(&project_dir, &["functions", "build"]);
    let artifact = fs::read_to_string(project_dir.join("functions/.kalam/build/module.js"))
        .expect("module.js");
    let manifest: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(project_dir.join("functions/.kalam/build/manifest.json"))
            .expect("manifest.json"),
    )
    .expect("parse manifest");
    let contract_hash = manifest.get("contractHash").and_then(|v| v.as_str()).unwrap_or_default();
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
    assert!((200..300).contains(&status), "second activate should succeed: {status} {body}");

    let nested = query_rows(&format!("CALL {ns}.plus_one(41)"));
    let nested_value = cell_i64(&nested[0], "result")
        .unwrap_or_else(|| cell(&nested[0], "result").as_i64().unwrap_or(-1));
    assert_eq!(
        nested_value, 42,
        "project-backed typed nested CALL should return 42: {nested:?}"
    );
    let greet = query_rows(&format!("CALL {ns}.greet()"));
    let greet_value = cell(&greet[0], "result");
    assert!(
        greet_value.as_str() == Some("project") || greet_value.to_string().contains("project"),
        "named override should beat inline: {greet:?}"
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
    assert!(elapsed_ms >= 40, "ctx.sleep(50) should pause the isolate: {elapsed_ms}ms");
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

/// Real SQL/transaction boundary: failures must release the runtime and roll back prior writes.
#[test]
#[ntest::timeout(30000)]
fn kobj_functions_runtime_failure_cleanup_and_recovery() {
    assert!(is_server_running(), "this test requires a running server");
    let ns = setup_namespace("fn_cleanup");
    let table = format!("{ns}.writes");
    exec(&format!(
        "CREATE TABLE {table} (id INT PRIMARY KEY, note TEXT) WITH (TYPE = 'SHARED')"
    ));
    ready(&table);
    create_js_procedure(
        &ns,
        "healthy",
        "",
        "globalThis.seen = (globalThis.seen || 0) + 1; return globalThis.seen;",
    );
    create_js_procedure(
        &ns,
        "timeout",
        "",
        &format!(
            "return ctx.db.execute('INSERT INTO {table} (id, note) VALUES ($1, $2)', [1, \
             'rollback']).then(() => ctx.sleep(6000));"
        ),
    );
    let started = std::time::Instant::now();
    let timeout = exec_err(&format!("CALL {ns}.timeout()"));
    assert!(
        timeout.to_lowercase().contains("time") || timeout.to_lowercase().contains("cancel"),
        "{timeout}"
    );
    assert!(query_rows(&format!("SELECT id FROM {table}")).is_empty());
    for (name, body) in [
        ("sparse", "return new Array(1000000);"),
        ("cycle", "const value = {}; value.self = value; return value;"),
        ("buffer", "return new ArrayBuffer(128 * 1024 * 1024).byteLength;"),
        ("reject", "Promise.reject(new Error('detached failure')); return 1;"),
    ] {
        create_js_procedure(&ns, name, "", body);
        for _ in 0..3 {
            let _ = exec_err(&format!("CALL {ns}.{name}()"));
            let healthy = query_rows(&format!("CALL {ns}.healthy()"));
            assert_eq!(
                cell_i64(&healthy[0], "result"),
                Some(1),
                "fresh invocation globals: {healthy:?}"
            );
        }
    }
    eprintln!(
        "function failure/rollback/recovery runtime_seconds={:.3}",
        started.elapsed().as_secs_f64()
    );
    exec(&format!("DROP NAMESPACE {ns} CASCADE"));
}

/// Concurrent parameter binding and per-call globals through the database API.
#[test]
#[ntest::timeout(30000)]
fn kobj_functions_runtime_concurrent_calls_preserve_inputs() {
    assert!(is_server_running(), "this test requires a running server");
    let ns = setup_namespace("fn_concurrent");
    create_js_procedure(
        &ns,
        "bound",
        "value INT",
        r#"
        globalThis.seen = (globalThis.seen || 0) + 1;
        if (globalThis.seen !== 1) throw new Error('cross-request state');
        return ctx.db.query('SELECT $1 AS value', [input.value]).then(rows => rows[0].value);
    "#,
    );
    let started = std::time::Instant::now();
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..2)
            .map(|worker| {
                let ns = &ns;
                scope.spawn(move || {
                    let mut latencies = Vec::new();
                    for index in 0..50 {
                        let expected = worker * 50 + index;
                        let start = std::time::Instant::now();
                        let rows = query_rows(&format!("CALL {ns}.bound({expected})"));
                        assert_eq!(cell_i64(&rows[0], "result"), Some(expected));
                        latencies.push(start.elapsed().as_secs_f64());
                    }
                    latencies
                })
            })
            .collect();
        let mut latencies: Vec<_> =
            workers.into_iter().flat_map(|worker| worker.join().unwrap()).collect();
        latencies.sort_by(f64::total_cmp);
        eprintln!(
            "function_calls=100 concurrency=2 runtime_seconds={:.3} calls_per_second={:.1} \
             p95_seconds={:.6} p99_seconds={:.6}",
            started.elapsed().as_secs_f64(),
            100.0 / started.elapsed().as_secs_f64(),
            latencies[94],
            latencies[98]
        );
    });
    exec(&format!("DROP NAMESPACE {ns} CASCADE"));
}
