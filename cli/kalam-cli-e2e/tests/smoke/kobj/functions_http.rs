//! App-developer coverage for procedure REST + `ctx.http` + JSON contracts.
//!
//! These tests exercise the surface an HTTP client uses to call procedures:
//! request headers/query, response status/headers, typed REST errors, JSON
//! in/out (including SQL `CAST(... AS JSON)`), nested HTTP rules, and
//! SECURITY DEFINER actor/principal.

use std::collections::HashMap;

use serde_json::{json, Value};

use crate::{common::*, kobj_helpers::*};

struct RestOutcome {
    status:  u16,
    headers: reqwest::header::HeaderMap,
    body:    String,
}

fn create_json_procedure(ns: &str, name: &str, params: &str, extras: &str, body: &str) {
    let extras = extras.trim();
    let extras = if extras.is_empty() {
        String::new()
    } else {
        format!("{extras} ")
    };
    exec(&format!(
        "CREATE OR REPLACE PROCEDURE {ns}.{name}({params}) RETURNS JSON {extras}LANGUAGE \
         JAVASCRIPT AS $$\n{body}\n$$"
    ));
}

fn root_token() -> String {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        get_access_token(default_username(), default_password())
            .await
            .expect("root token")
    })
}

fn user_token(username: &str, password: &str) -> String {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        get_access_token(username, password)
            .await
            .unwrap_or_else(|err| panic!("token for {username}: {err}"))
    })
}

fn rest_invoke(
    path: &str,
    body: Value,
    token: Option<&str>,
    headers: &[(&str, &str)],
    query: &[(&str, &str)],
) -> RestOutcome {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let mut url = format!("{}{path}", server_url());
        if !query.is_empty() {
            let encoded = query
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>()
                .join("&");
            url.push('?');
            url.push_str(&encoded);
        }
        let mut request = shared_http_client().post(url).json(&body);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = request.send().await.expect("HTTP POST");
        RestOutcome {
            status:  response.status().as_u16(),
            headers: response.headers().clone(),
            body:    response.text().await.expect("HTTP body"),
        }
    })
}

fn rest_root(path: &str, body: Value) -> RestOutcome {
    let token = root_token();
    rest_invoke(path, body, Some(&token), &[], &[])
}

fn header_value(headers: &reqwest::header::HeaderMap, name: &str) -> Option<String> {
    headers.get(name).and_then(|value| value.to_str().ok()).map(ToOwned::to_owned)
}

fn parse_json_body(body: &str) -> Value {
    serde_json::from_str(body).unwrap_or_else(|err| panic!("JSON body should parse: {err}: {body}"))
}

fn error_code(body: &str) -> String {
    let parsed = parse_json_body(body);
    parsed
        .get("code")
        .and_then(Value::as_str)
        .or_else(|| parsed.pointer("/error/code").and_then(Value::as_str))
        .unwrap_or_default()
        .to_string()
}

fn json_result(row: &HashMap<String, Value>) -> Value {
    match cell(row, "result") {
        Value::String(text) => serde_json::from_str(&text).unwrap_or(Value::String(text)),
        other => other,
    }
}

#[ntest::timeout(180000)]
#[test]
fn kobj_functions_http_context_for_app_clients() {
    if skip_if_no_server() {
        return;
    }
    let ns = setup_namespace("kobj_http");
    let token = root_token();

    create_json_procedure(
        &ns,
        "inspect",
        "",
        "",
        r#"
ctx.http.response.status(201);
ctx.http.response.header('x-kalam-trace', 'ok');
ctx.http.response.header('Location', '/created');
ctx.http.response.contentType('application/json');
return {
  method: ctx.http.request.method,
  path: ctx.http.request.path,
  version: ctx.http.request.headers.get('x-client-version'),
  acceptLanguage: ctx.http.request.headers.get('accept-language'),
  authorization: ctx.http.request.headers.get('Authorization'),
  cookie: ctx.http.request.headers.get('Cookie'),
  missing: ctx.http.request.headers.get('x-not-sent'),
  lang: ctx.http.request.query.get('lang'),
  unused: ctx.http.request.query.get('unused'),
  source: ctx.source.kind,
  parent: ctx.parent,
  http: ctx.http === null ? 'null' : 'present',
  actor: ctx.actor.id,
  principal: ctx.principal.id
};
"#,
    );
    create_json_procedure(
        &ns,
        "sql_probe",
        "",
        "",
        "return { http: ctx.http === null ? 'null' : 'present', source: ctx.source.kind, parent: \
         ctx.parent };",
    );
    create_json_procedure(
        &ns,
        "child_read",
        "",
        "",
        "return { version: ctx.http.request.headers.get('x-client-version'), parent: ctx.parent, \
         http: ctx.http === null ? 'null' : 'present' };",
    );
    create_json_procedure(
        &ns,
        "parent_read",
        "",
        "",
        &format!("return ctx.functions.call('{ns}.child_read');"),
    );
    create_json_procedure(
        &ns,
        "nested_set",
        "",
        "",
        "ctx.http.response.status(500); return { ok: true };",
    );
    create_json_procedure(
        &ns,
        "root_set",
        "",
        "",
        &format!("return ctx.functions.call('{ns}.nested_set');"),
    );
    create_json_procedure(
        &ns,
        "bad_header",
        "",
        "",
        "ctx.http.response.header('Connection', 'close'); return { ok: true };",
    );
    create_json_procedure(
        &ns,
        "bad_status",
        "",
        "",
        "ctx.http.response.status(99); return { ok: true };",
    );
    create_json_procedure(
        &ns,
        "huge_header",
        "",
        "",
        "ctx.http.response.header('x-big', 'a'.repeat(20000)); return { ok: true };",
    );

    let inspect = rest_invoke(
        &format!("/v1/functions/{ns}/inspect"),
        json!({}),
        Some(&token),
        &[
            ("x-client-version", "sdk-9"),
            ("accept-language", "fr"),
            ("Cookie", "session=secret"),
        ],
        &[("lang", "fr"), ("unused", "1")],
    );
    assert_eq!(inspect.status, 201, "root status() should set HTTP status: {}", inspect.body);
    assert_eq!(header_value(&inspect.headers, "x-kalam-trace").as_deref(), Some("ok"));
    assert_eq!(header_value(&inspect.headers, "location").as_deref(), Some("/created"));
    let content_type = header_value(&inspect.headers, "content-type").unwrap_or_default();
    assert!(
        content_type.to_ascii_lowercase().contains("application/json"),
        "contentType() should keep JSON: {content_type}"
    );
    let seen = parse_json_body(&inspect.body);
    assert_eq!(seen["method"], "POST", "{seen}");
    assert!(
        seen["path"]
            .as_str()
            .is_some_and(|path| path.ends_with(&format!("/functions/{ns}/inspect"))),
        "path should be the REST route: {seen}"
    );
    assert_eq!(seen["version"], "sdk-9", "{seen}");
    assert_eq!(seen["acceptLanguage"], "fr", "{seen}");
    assert!(seen["authorization"].is_null(), "Authorization must not be readable: {seen}");
    assert!(seen["cookie"].is_null(), "Cookie must not be readable: {seen}");
    assert!(seen["missing"].is_null(), "missing headers are null: {seen}");
    assert_eq!(seen["lang"], "fr", "{seen}");
    assert_eq!(seen["unused"], "1", "{seen}");
    assert_eq!(seen["source"], "call", "{seen}");
    assert!(seen["parent"].is_null(), "root parent is null: {seen}");
    assert_eq!(seen["http"], "present", "{seen}");
    assert_eq!(seen["actor"], seen["principal"], "HTTP invoker actor=principal: {seen}");

    let sql_probe = query_rows(&format!("CALL {ns}.sql_probe()"));
    let sql_seen = json_result(&sql_probe[0]);
    assert_eq!(sql_seen["http"], "null", "SQL CALL must set ctx.http to null: {sql_seen}");
    assert_eq!(sql_seen["source"], "call", "{sql_seen}");
    assert!(sql_seen["parent"].is_null(), "{sql_seen}");

    let nested_read = rest_invoke(
        &format!("/v1/functions/{ns}/parent_read"),
        json!({}),
        Some(&token),
        &[("x-client-version", "nested-ok")],
        &[],
    );
    assert!(
        (200..300).contains(&nested_read.status),
        "nested request reads should succeed: {} {}",
        nested_read.status,
        nested_read.body
    );
    let nested_seen = parse_json_body(&nested_read.body);
    assert_eq!(
        nested_seen["version"], "nested-ok",
        "nested frames may read request: {nested_seen}"
    );
    assert_eq!(nested_seen["http"], "present", "{nested_seen}");
    let parent = nested_seen["parent"].as_str().unwrap_or_default();
    assert!(
        parent.contains("parent_read"),
        "nested ctx.parent should name the caller: {nested_seen}"
    );

    let nested_mutate =
        rest_invoke(&format!("/v1/functions/{ns}/root_set"), json!({}), Some(&token), &[], &[]);
    assert_eq!(
        nested_mutate.status, 400,
        "nested response mutation must fail: {}",
        nested_mutate.body
    );
    let nested_err = nested_mutate.body.to_ascii_lowercase();
    assert!(
        nested_err.contains("nested") || nested_err.contains("ctx.http"),
        "nested mutation error should mention ctx.http: {}",
        nested_mutate.body
    );

    let hop = rest_root(&format!("/v1/functions/{ns}/bad_header"), json!({}));
    assert_eq!(hop.status, 400, "hop-by-hop response headers must be rejected: {}", hop.body);
    assert!(
        hop.body.to_ascii_lowercase().contains("connection")
            || hop.body.to_ascii_lowercase().contains("not allowed"),
        "rejected header name should appear: {}",
        hop.body
    );

    let bad_status = rest_root(&format!("/v1/functions/{ns}/bad_status"), json!({}));
    assert_eq!(bad_status.status, 400, "invalid status must fail: {}", bad_status.body);

    let huge = rest_root(&format!("/v1/functions/{ns}/huge_header"), json!({}));
    assert_eq!(huge.status, 429, "oversized response header is RESOURCE_LIMIT: {}", huge.body);
    assert_eq!(error_code(&huge.body), "RESOURCE_LIMIT", "{}", huge.body);
}

#[ntest::timeout(180000)]
#[test]
fn kobj_functions_rest_json_errors_and_sql_cast() {
    if skip_if_no_server() {
        return;
    }
    let ns = setup_namespace("kobj_http_json");
    let token = root_token();

    create_json_procedure(
        &ns,
        "echo_json",
        "body JSON NOT NULL",
        "",
        "return { ok: true, conversationId: input.body.conversationId, text: input.body.text, \
         extra: input.body.ignored };",
    );
    exec(&format!(
        "CREATE OR REPLACE PROCEDURE {ns}.echo_text(msg TEXT) LANGUAGE JAVASCRIPT AS $$\n  return \
         input.msg;\n$$"
    ));
    exec(&format!(
        "CREATE OR REPLACE PROCEDURE {ns}.need_two(first TEXT, last TEXT) LANGUAGE JAVASCRIPT AS \
         $$\n  return input.first + ' ' + input.last;\n$$"
    ));

    let whole = rest_invoke(
        &format!("/v1/functions/{ns}/echo_json"),
        json!({ "conversationId": "c1", "text": "hello", "ignored": "drop-me" }),
        Some(&token),
        &[],
        &[],
    );
    assert!(
        (200..300).contains(&whole.status),
        "whole-body JSON REST should succeed: {} {}",
        whole.status,
        whole.body
    );
    let parsed = parse_json_body(&whole.body);
    assert_eq!(parsed["ok"], true, "{parsed}");
    assert_eq!(parsed["conversationId"], "c1", "{parsed}");
    assert_eq!(parsed["text"], "hello", "{parsed}");

    let wrapped = rest_root(
        &format!("/v1/functions/{ns}/echo_json"),
        json!({ "body": { "conversationId": "c2", "text": "named" } }),
    );
    let parsed = parse_json_body(&wrapped.body);
    assert_eq!(parsed["conversationId"], "c2", "{parsed}");
    assert_eq!(parsed["text"], "named", "{parsed}");

    let positional = rest_root(
        &format!("/v1/functions/{ns}/echo_json"),
        json!([{ "conversationId": "c3", "text": "pos" }]),
    );
    let parsed = parse_json_body(&positional.body);
    assert_eq!(parsed["conversationId"], "c3", "{parsed}");
    assert_eq!(parsed["text"], "pos", "{parsed}");

    let sql_rows = query_rows(&format!(
        "CALL {ns}.echo_json(CAST('{{\"conversationId\":\"c4\",\"text\":\"sql\"}}' AS JSON))"
    ));
    let sql_parsed = json_result(&sql_rows[0]);
    assert_eq!(
        sql_parsed["conversationId"], "c4",
        "SQL CAST JSON must bind as an object: {sql_parsed}"
    );
    assert_eq!(sql_parsed["text"], "sql", "{sql_parsed}");

    let unauth = rest_invoke(&format!("/v1/functions/{ns}/echo_json"), json!({}), None, &[], &[]);
    assert_eq!(unauth.status, 401, "missing bearer must be 401: {}", unauth.body);
    let unauth_code = error_code(&unauth.body);
    assert!(
        unauth_code == "MISSING_AUTHORIZATION" || unauth_code == "AUTHENTICATION_REQUIRED",
        "unauthenticated REST code: {unauth_code} {}",
        unauth.body
    );

    let missing = rest_root(&format!("/v1/functions/{ns}/does_not_exist"), json!({}));
    assert_eq!(missing.status, 404, "{}", missing.body);
    assert_eq!(error_code(&missing.body), "PROCEDURE_NOT_FOUND", "{}", missing.body);

    let unimplemented = exec(&format!("CREATE PROCEDURE {ns}.later() RETURNS JSON"));
    assert!(unimplemented.to_ascii_lowercase().contains("created"), "{unimplemented}");
    let not_impl = rest_root(&format!("/v1/functions/{ns}/later"), json!({}));
    assert_eq!(not_impl.status, 404, "{}", not_impl.body);
    assert_eq!(error_code(&not_impl.body), "PROCEDURE_NOT_IMPLEMENTED", "{}", not_impl.body);

    let (username, password) = create_login_user("httpe2e");
    let user = user_token(&username, &password);
    let denied = rest_invoke(
        &format!("/v1/functions/{ns}/echo_text"),
        json!({ "msg": "nope" }),
        Some(&user),
        &[],
        &[],
    );
    assert_eq!(denied.status, 403, "user without EXECUTE is 403: {}", denied.body);
    assert_eq!(error_code(&denied.body), "EXECUTE_DENIED", "{}", denied.body);

    exec(&format!("GRANT EXECUTE ON PROCEDURE {ns}.echo_text TO user"));
    let granted = rest_invoke(
        &format!("/v1/functions/{ns}/echo_text"),
        json!({ "msg": "ok" }),
        Some(&user),
        &[],
        &[],
    );
    assert!(
        (200..300).contains(&granted.status),
        "granted user REST should succeed: {} {}",
        granted.status,
        granted.body
    );
    assert!(granted.body.contains("ok"), "{}", granted.body);

    let bad_args = rest_root(&format!("/v1/functions/{ns}/need_two"), json!({ "first": "Ada" }));
    assert_eq!(bad_args.status, 400, "{}", bad_args.body);
    assert_eq!(error_code(&bad_args.body), "INVALID_ARGUMENTS", "{}", bad_args.body);

    let context_key =
        rest_root(&format!("/v1/functions/{ns}/echo_text"), json!({ "ctx": {}, "msg": "x" }));
    assert_eq!(context_key.status, 400, "{}", context_key.body);
    assert_eq!(error_code(&context_key.body), "INVALID_ARGUMENTS", "{}", context_key.body);
    assert!(
        context_key.body.contains("ctx"),
        "client-supplied ctx must be rejected: {}",
        context_key.body
    );
}

#[ntest::timeout(180000)]
#[test]
fn kobj_functions_http_definer_actor_and_db() {
    if skip_if_no_server() {
        return;
    }
    let ns = setup_namespace("kobj_http_acl");
    let docs = format!("{ns}.docs");
    exec(&format!(
        "CREATE TABLE {docs} (id INT PRIMARY KEY, note TEXT) WITH (TYPE = 'SHARED')"
    ));
    grant_public_select_shared_table(&docs);
    ready(&docs);

    create_json_procedure(
        &ns,
        "whoami",
        "",
        "",
        "return { actor: ctx.actor.id, principal: ctx.principal.id, role: ctx.actor.role, source: \
         ctx.source.kind };",
    );
    create_json_procedure(
        &ns,
        "write_invoker",
        "id INT",
        "",
        &format!(
            "return ctx.db.execute('INSERT INTO {docs} (id, note) VALUES ($1, $2)', [input.id, \
             ctx.actor.id]).then(function () {{ return {{ actor: ctx.actor.id, principal: \
             ctx.principal.id }}; }});"
        ),
    );
    create_json_procedure(
        &ns,
        "write_definer",
        "id INT",
        "SECURITY DEFINER",
        &format!(
            "return ctx.db.execute('INSERT INTO {docs} (id, note) VALUES ($1, $2)', [input.id, \
             ctx.actor.id]).then(function () {{ return {{ actor: ctx.actor.id, principal: \
             ctx.principal.id }}; }});"
        ),
    );
    create_json_procedure(
        &ns,
        "lookup",
        "n INT",
        "",
        "return ctx.db.query('SELECT $1 AS n', [input.n]).then(function (rows) { return { n: \
         rows[0].n }; });",
    );
    exec(&format!("GRANT EXECUTE ON PROCEDURE {ns}.whoami TO user"));
    exec(&format!("GRANT EXECUTE ON PROCEDURE {ns}.write_invoker TO user"));
    exec(&format!("GRANT EXECUTE ON PROCEDURE {ns}.write_definer TO user"));
    exec(&format!("GRANT EXECUTE ON PROCEDURE {ns}.lookup TO user"));

    let (username, password) = create_login_user("httpacl");
    let user = user_token(&username, &password);

    let who = rest_invoke(&format!("/v1/functions/{ns}/whoami"), json!({}), Some(&user), &[], &[]);
    assert!((200..300).contains(&who.status), "{} {}", who.status, who.body);
    let identity = parse_json_body(&who.body);
    assert_eq!(identity["actor"], username, "{identity}");
    assert_eq!(identity["principal"], username, "INVOKER principal is the caller: {identity}");
    assert_eq!(identity["source"], "call", "{identity}");

    let invoker = rest_invoke(
        &format!("/v1/functions/{ns}/write_invoker"),
        json!({ "id": 1 }),
        Some(&user),
        &[],
        &[],
    );
    assert!(
        invoker.status >= 400,
        "INVOKER without table write must fail: {} {}",
        invoker.status,
        invoker.body
    );
    assert_eq!(count_sql(&format!("SELECT COUNT(*) FROM {docs} WHERE id = 1")), 0);

    let definer = rest_invoke(
        &format!("/v1/functions/{ns}/write_definer"),
        json!({ "id": 2 }),
        Some(&user),
        &[],
        &[],
    );
    assert!(
        (200..300).contains(&definer.status),
        "DEFINER should write as the owner: {} {}",
        definer.status,
        definer.body
    );
    let written = parse_json_body(&definer.body);
    assert_eq!(written["actor"], username, "DEFINER keeps the caller as actor: {written}");
    assert_ne!(
        written["principal"].as_str().unwrap_or_default(),
        username,
        "DEFINER principal is the owner, not the caller: {written}"
    );
    assert_eq!(count_sql(&format!("SELECT COUNT(*) FROM {docs} WHERE id = 2")), 1);

    let lookup = rest_invoke(
        &format!("/v1/functions/{ns}/lookup"),
        json!({ "n": 7 }),
        Some(&user),
        &[],
        &[],
    );
    assert!((200..300).contains(&lookup.status), "{} {}", lookup.status, lookup.body);
    let rows = parse_json_body(&lookup.body);
    assert_eq!(rows["n"], 7, "ctx.db.query params should round-trip over REST: {rows}");
}
