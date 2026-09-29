//! Cluster coverage for the function admission pool and client cancellation.
//!
//! Defaults, with no `[functions]` override in the local cluster config:
//! 16 active calls, 128 queued, then `function engine capacity exhausted`.
//! A dropped HTTP client must stop the procedure on the leader, including when
//! the client was talking to a follower. The insert after `ctx.sleep` is the
//! proof that the call did not keep running until the sleep or the timeout.

use std::{
    io::Write,
    net::{Shutdown, TcpStream},
    sync::atomic::{AtomicUsize, Ordering},
    thread,
    time::{Duration, Instant},
};

use serde_json::Value;

use crate::{cluster_common::*, common::*};

const MAX_ACTIVE: usize = 16;
const MAX_QUEUED: usize = 128;

fn leader_and_followers() -> (String, Vec<String>) {
    let urls = cluster_urls();
    let leader = leader_url().expect("cluster leader URL");
    let followers: Vec<String> = urls.into_iter().filter(|url| url != &leader).collect();
    assert!(!followers.is_empty(), "function runtime tests need at least one follower");
    (leader, followers)
}

fn nodes_from(leader: &str, followers: &[String]) -> Vec<String> {
    let mut nodes = Vec::with_capacity(1 + followers.len());
    nodes.push(leader.to_string());
    nodes.extend(followers.iter().cloned());
    nodes
}

fn setup_namespace(leader: &str, label: &str) -> String {
    let namespace = generate_unique_namespace(label);
    let _ = execute_on_node(leader, &format!("DROP NAMESPACE IF EXISTS {namespace} CASCADE"));
    execute_on_node(leader, &format!("CREATE NAMESPACE {namespace}"))
        .unwrap_or_else(|err| panic!("create namespace {namespace}: {err}"));
    namespace
}

fn create_js_procedure(node: &str, namespace: &str, name: &str, params: &str, body: &str) {
    let mut sql = format!(
        "CREATE OR REPLACE PROCEDURE {namespace}.{name}({params}) LANGUAGE JAVASCRIPT AS $$"
    );
    sql.push_str(body);
    sql.push_str("$$");
    execute_on_node_raw(node, &sql)
        .unwrap_or_else(|err| panic!("create {namespace}.{name} on {node}: {err}"));
}

fn cell_i64(value: &Value) -> i64 {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|n| i64::try_from(n).ok()))
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
        .unwrap_or_else(|| panic!("expected an integer cell, got {value}"))
}

fn call_i64(node: &str, sql: &str) -> Result<i64, String> {
    let response = execute_on_node_response_raw(node, sql)?;
    let cell = response
        .results
        .first()
        .and_then(|result| result.rows.as_ref())
        .and_then(|rows| rows.first())
        .and_then(|row| row.first())
        .ok_or_else(|| format!("CALL returned no cell: {sql}"))?;
    Ok(cell_i64(&extract_typed_value(cell)))
}

fn active_count(leader: &str, procedure_id: &str) -> Result<usize, String> {
    let sql = format!(
        "SELECT execution_id FROM system.active_procedure_runs WHERE procedure_id = \
         '{procedure_id}'"
    );
    let response = execute_on_node_response(leader, &sql)?;
    Ok(response
        .results
        .first()
        .and_then(|result| result.rows.as_ref())
        .map(|rows| rows.len())
        .unwrap_or(0))
}

fn active_ids(leader: &str, procedure_id: &str) -> Result<Vec<String>, String> {
    let sql = format!(
        "SELECT execution_id FROM system.active_procedure_runs WHERE procedure_id = \
         '{procedure_id}'"
    );
    let response = execute_on_node_response(leader, &sql)?;
    let Some(rows) = response.results.first().and_then(|result| result.rows.as_ref()) else {
        return Ok(Vec::new());
    };
    Ok(rows
        .iter()
        .filter_map(|row| row.first())
        .map(|cell| {
            let value = extract_typed_value(cell);
            value.as_str().map(str::to_string).unwrap_or_else(|| value.to_string())
        })
        .collect())
}

fn bearer(base_url: &str) -> String {
    cluster_runtime()
        .block_on(get_access_token_for_url(base_url, default_username(), default_password()))
        .unwrap_or_else(|err| panic!("access token for {base_url}: {err}"))
}

fn host_port(base_url: &str) -> (String, u16) {
    let without = base_url.trim_start_matches("https://").trim_start_matches("http://");
    let host_port = without.split('/').next().unwrap_or(without);
    let (host, port) = host_port.rsplit_once(':').unwrap_or((host_port, "80"));
    (host.to_string(), port.parse().unwrap_or_else(|_| panic!("port in {base_url}")))
}

/// Writes one SQL request and returns the socket still open.
fn open_sql_call(base_url: &str, token: &str, sql: &str) -> TcpStream {
    let (host, port) = host_port(base_url);
    let body = serde_json::json!({ "sql": sql }).to_string();
    let mut request = format!(
        "POST /v1/api/sql HTTP/1.1\r\nHost: {host}:{port}\r\nAuthorization: Bearer \
         {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: \
         close\r\n\r\n",
        body.len()
    );
    request.push_str(&body);
    let mut stream = TcpStream::connect((host.as_str(), port))
        .unwrap_or_else(|err| panic!("connect {base_url}: {err}"));
    stream
        .write_all(request.as_bytes())
        .unwrap_or_else(|err| panic!("write SQL to {base_url}: {err}"));
    stream.flush().unwrap_or_else(|err| panic!("flush SQL to {base_url}: {err}"));
    stream
}

struct CallOutcome {
    id:      i64,
    elapsed: Duration,
    result:  Result<i64, String>,
}

fn burst_calls(nodes: &[String], namespace: &str, procedure: &str, count: i64) -> Vec<CallOutcome> {
    thread::scope(|scope| {
        let mut handles = Vec::with_capacity(count as usize);
        for id in 1..=count {
            let node = nodes[((id - 1) as usize) % nodes.len()].clone();
            let sql = format!("CALL {namespace}.{procedure}({id})");
            handles.push(scope.spawn(move || {
                let started = Instant::now();
                let result = call_i64(&node, &sql);
                CallOutcome {
                    id,
                    elapsed: started.elapsed(),
                    result,
                }
            }));
        }
        handles
            .into_iter()
            .map(|handle| handle.join().expect("call thread panicked"))
            .collect()
    })
}

/// Sixteen calls of one procedure run at the same time, from the leader and
/// from followers, and each returns its own argument.
#[ntest::timeout(180_000)]
#[test]
fn cluster_test_function_pool_overlaps_same_procedure() {
    if !require_cluster_running() {
        return;
    }
    let _function_pool = lock_function_pool();

    let (leader, followers) = leader_and_followers();
    let nodes = nodes_from(&leader, &followers);
    let namespace = setup_namespace(&leader, "fn_overlap");
    create_js_procedure(
        &leader,
        &namespace,
        "hold",
        "id INT",
        "return ctx.sleep(1200).then(function () { return input.id; });",
    );
    let procedure_id = format!("{namespace}.hold");
    let warmup = call_i64(&leader, &format!("CALL {procedure_id}(0)"));
    assert_eq!(warmup.as_ref().ok().copied(), Some(0), "warmup failed: {warmup:?}");

    let peak = AtomicUsize::new(0);
    let peak_ref = &peak;
    let started = Instant::now();
    let outcomes = thread::scope(|scope| {
        let leader_for_sample = leader.clone();
        let procedure_for_sample = procedure_id.clone();
        scope.spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(4);
            while Instant::now() < deadline {
                if let Ok(count) = active_count(&leader_for_sample, &procedure_for_sample) {
                    peak_ref.fetch_max(count, Ordering::Relaxed);
                    if count >= MAX_ACTIVE {
                        break;
                    }
                }
                thread::sleep(Duration::from_millis(30));
            }
        });
        burst_calls(&nodes, &namespace, "hold", MAX_ACTIVE as i64)
    });

    assert!(
        started.elapsed() < Duration::from_secs(8),
        "16 overlapping calls took {:?}; they should share the pool, not run one after another",
        started.elapsed()
    );
    assert_eq!(
        peak.load(Ordering::Relaxed),
        MAX_ACTIVE,
        "system.active_procedure_runs should show all {MAX_ACTIVE} calls of {procedure_id} at once"
    );
    for outcome in &outcomes {
        match &outcome.result {
            Ok(value) => assert_eq!(*value, outcome.id, "call {} returned {value}", outcome.id),
            Err(err) => panic!("overlapping call {} failed: {err}", outcome.id),
        }
    }
}

/// The pool admits 16, queues 128, and rejects the rest immediately.
#[ntest::timeout(180_000)]
#[test]
fn cluster_test_function_pool_queues_then_rejects() {
    if !require_cluster_running() {
        return;
    }
    let _function_pool = lock_function_pool();

    let (leader, followers) = leader_and_followers();
    let nodes = nodes_from(&leader, &followers);
    let namespace = setup_namespace(&leader, "fn_queue");
    create_js_procedure(
        &leader,
        &namespace,
        "hold",
        "id INT",
        "return ctx.sleep(300).then(function () { return input.id; });",
    );
    let procedure_id = format!("{namespace}.hold");
    assert!(
        call_i64(&followers[0], &format!("CALL {procedure_id}(0)")).is_ok(),
        "warmup CALL from a follower failed"
    );

    let total = (MAX_ACTIVE + MAX_QUEUED + 8) as i64;
    let started = Instant::now();
    let outcomes = burst_calls(&nodes, &namespace, "hold", total);
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "queued calls took {:?}; 144 admissions of a 300ms procedure should finish well under the \
         5s timeout",
        started.elapsed()
    );

    let mut admitted = 0usize;
    let mut rejected = 0usize;
    for outcome in &outcomes {
        match &outcome.result {
            Ok(value) => {
                assert_eq!(*value, outcome.id, "admitted call {} returned {value}", outcome.id);
                admitted += 1;
            },
            Err(err) => {
                let message = err.to_ascii_lowercase();
                assert!(
                    message.contains("capacity"),
                    "only overflow should fail, call {} returned {err} after {:?}",
                    outcome.id,
                    outcome.elapsed
                );
                assert!(
                    outcome.elapsed < Duration::from_millis(1500),
                    "capacity rejection for call {} took {:?}; it should fail without waiting for \
                     a slot",
                    outcome.id,
                    outcome.elapsed
                );
                rejected += 1;
            },
        }
    }
    assert_eq!(admitted, MAX_ACTIVE + MAX_QUEUED, "admitted {admitted}, rejected {rejected}");
    assert_eq!(rejected, 8, "the calls past the queue should be the only failures");
}

/// Closing the leader connection stops a running procedure and frees its slot
/// for a call that was waiting.
#[ntest::timeout(180_000)]
#[test]
fn cluster_test_function_cancel_on_leader_frees_slot() {
    if !require_cluster_running() {
        return;
    }
    let _function_pool = lock_function_pool();

    let (leader, _) = leader_and_followers();
    let namespace = setup_namespace(&leader, "fn_cancel_leader");
    create_js_procedure(
        &leader,
        &namespace,
        "hold",
        "id INT",
        "return ctx.sleep(2000).then(function () { return input.id; });",
    );
    create_js_procedure(
        &leader,
        &namespace,
        "quick",
        "id INT",
        "return ctx.sleep(50).then(function () { return input.id; });",
    );
    let hold_id = format!("{namespace}.hold");
    assert!(call_i64(&leader, &format!("CALL {namespace}.quick(0)")).is_ok());

    let token = bearer(&leader);
    let mut streams = Vec::with_capacity(MAX_ACTIVE);
    for id in 1..=MAX_ACTIVE as i64 {
        streams.push(open_sql_call(&leader, &token, &format!("CALL {namespace}.hold({id})")));
    }

    let mut saw_full = false;
    let wait_deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < wait_deadline {
        if active_count(&leader, &hold_id).unwrap_or(0) >= MAX_ACTIVE {
            saw_full = true;
            break;
        }
        thread::sleep(Duration::from_millis(30));
    }
    assert!(saw_full, "expected {MAX_ACTIVE} active runs of {hold_id} within 2s of launch");

    let quick_leader = leader.clone();
    let quick_sql = format!("CALL {namespace}.quick(99)");
    let quick_started = Instant::now();
    let quick = thread::spawn(move || call_i64(&quick_leader, &quick_sql));
    thread::sleep(Duration::from_millis(80));
    streams[0]
        .shutdown(Shutdown::Both)
        .unwrap_or_else(|err| panic!("close leader call: {err}"));

    let quick_result = quick.join().expect("quick call thread panicked");
    assert_eq!(
        quick_result.as_ref().ok().copied(),
        Some(99),
        "queued call failed: {quick_result:?}"
    );
    assert!(
        quick_started.elapsed() < Duration::from_millis(700),
        "queued call took {:?} after the cancelled call should have released its slot",
        quick_started.elapsed()
    );

    let mut remaining = MAX_ACTIVE;
    let clear_deadline = Instant::now() + Duration::from_millis(800);
    while Instant::now() < clear_deadline {
        remaining = active_count(&leader, &hold_id).unwrap_or(MAX_ACTIVE);
        if remaining < MAX_ACTIVE {
            break;
        }
        thread::sleep(Duration::from_millis(30));
    }
    assert!(
        remaining < MAX_ACTIVE,
        "cancelled leader call is still occupying an active slot"
    );
    assert!(
        remaining > 0,
        "every hold ended when one socket closed; the others should keep sleeping"
    );

    let mut finished = false;
    let finish_deadline = Instant::now() + Duration::from_millis(2500);
    while Instant::now() < finish_deadline {
        if active_count(&leader, &hold_id).unwrap_or(1) == 0 {
            finished = true;
            break;
        }
        thread::sleep(Duration::from_millis(40));
    }
    assert!(
        finished,
        "a hold was still active after the sleep window; the closed call should already have \
         stopped and the others should have finished"
    );
    drop(streams);
}

/// A follower disconnect cancels the leader procedure, including a nested call
/// and a call that was only queued.
#[ntest::timeout(180_000)]
#[test]
fn cluster_test_function_cancel_on_follower_stops_leader() {
    if !require_cluster_running() {
        return;
    }
    let _function_pool = lock_function_pool();

    let (leader, followers) = leader_and_followers();
    let follower = &followers[0];
    let namespace = setup_namespace(&leader, "fn_cancel_follower");
    create_js_procedure(
        &leader,
        &namespace,
        "child",
        "id INT",
        "return ctx.sleep(2000).then(function () { return input.id; });",
    );
    create_js_procedure(
        &leader,
        &namespace,
        "hold",
        "id INT",
        "return ctx.sleep(2000).then(function () { return input.id; });",
    );
    create_js_procedure(
        &leader,
        &namespace,
        "parent",
        "id INT",
        &format!("return ctx.functions.call('{namespace}.child', [input]);"),
    );
    create_js_procedure(&leader, &namespace, "ping", "id INT", "return input.id;");
    let parent_id = format!("{namespace}.parent");
    assert_eq!(
        call_i64(follower, &format!("CALL {namespace}.ping(0)")).ok(),
        Some(0),
        "warmup CALL from the follower failed"
    );

    let token = bearer(&leader);
    let running = open_sql_call(follower, &token, &format!("CALL {namespace}.parent(1)"));
    let mut became_active = false;
    let active_deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < active_deadline {
        if active_count(&leader, &parent_id).unwrap_or(0) >= 1 {
            became_active = true;
            break;
        }
        thread::sleep(Duration::from_millis(30));
    }
    assert!(became_active, "follower CALL did not become an active run on the leader");
    let running_started = Instant::now();
    running
        .shutdown(Shutdown::Both)
        .unwrap_or_else(|err| panic!("close follower call: {err}"));

    let mut cleared = false;
    let clear_deadline = Instant::now() + Duration::from_millis(800);
    while Instant::now() < clear_deadline {
        if active_count(&leader, &parent_id).unwrap_or(1) == 0 {
            cleared = true;
            break;
        }
        thread::sleep(Duration::from_millis(30));
    }
    assert!(
        cleared,
        "nested call started from a follower was still active {}ms after disconnect",
        running_started.elapsed().as_millis()
    );
    thread::sleep(Duration::from_millis(2200));
    assert_eq!(
        active_count(&leader, &parent_id).unwrap_or(1),
        0,
        "nested call became active again after the follower disconnected"
    );

    let hold_id = format!("{namespace}.hold");
    let mut holders = Vec::with_capacity(MAX_ACTIVE);
    for id in 10..10 + MAX_ACTIVE as i64 {
        holders.push(open_sql_call(&leader, &token, &format!("CALL {namespace}.hold({id})")));
    }
    let mut pool_full = false;
    let full_deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < full_deadline {
        if active_count(&leader, &hold_id).unwrap_or(0) >= MAX_ACTIVE {
            pool_full = true;
            break;
        }
        thread::sleep(Duration::from_millis(30));
    }
    assert!(pool_full, "could not fill the pool before the queued-cancel check");
    let running_ids = active_ids(&leader, &hold_id).unwrap_or_default();
    assert_eq!(running_ids.len(), MAX_ACTIVE, "holder execution ids: {running_ids:?}");

    let queued = open_sql_call(follower, &token, &format!("CALL {namespace}.hold(50)"));
    let mut queued_id = None;
    let appear_deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < appear_deadline {
        let ids = active_ids(&leader, &hold_id).unwrap_or_default();
        if let Some(foreign) = ids.into_iter().find(|id| !running_ids.contains(id)) {
            queued_id = Some(foreign);
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    let queued_id =
        queued_id.expect("queued follower call never showed up in active_procedure_runs");
    queued
        .shutdown(Shutdown::Both)
        .unwrap_or_else(|err| panic!("close queued follower call: {err}"));
    drop(queued);

    let mut queued_cleared = false;
    let clear_queued = Instant::now() + Duration::from_millis(800);
    while Instant::now() < clear_queued {
        let ids = active_ids(&leader, &hold_id).unwrap_or_default();
        let holders_still_running = ids.iter().any(|id| running_ids.contains(id));
        if !ids.iter().any(|id| id == &queued_id) && holders_still_running {
            queued_cleared = true;
            break;
        }
        thread::sleep(Duration::from_millis(30));
    }
    assert!(
        queued_cleared,
        "queued follower call {queued_id} was still running after disconnect"
    );

    let mut saw_idle = false;
    let mut returned = false;
    let watch_until = Instant::now() + Duration::from_secs(4);
    while Instant::now() < watch_until {
        let ids = active_ids(&leader, &hold_id).unwrap_or_default();
        if ids.iter().any(|id| id == &queued_id) {
            returned = true;
            break;
        }
        if ids.is_empty() {
            saw_idle = true;
            thread::sleep(Duration::from_millis(400));
            let ids = active_ids(&leader, &hold_id).unwrap_or_default();
            returned = ids.iter().any(|id| id == &queued_id);
            break;
        }
        thread::sleep(Duration::from_millis(40));
    }
    assert!(!returned, "queued follower call started after its client disconnected");
    assert!(saw_idle, "the holder calls never finished");
    drop(holders);
}
