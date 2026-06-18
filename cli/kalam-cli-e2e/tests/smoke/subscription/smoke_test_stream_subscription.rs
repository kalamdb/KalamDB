// Smoke Test 4: Stream table subscription
// Covers: namespace creation, stream table creation with TTL, subscription, insert, receive event

use crate::common::*;

#[ntest::timeout(180000)]
#[test]
fn smoke_stream_table_subscription() {
    if !is_server_running() {
        println!(
            "Skipping smoke_stream_table_subscription: server not running at {}",
            server_url()
        );
        return;
    }

    // Unique per run
    let namespace = generate_unique_namespace("smoke_ns");
    let table = generate_unique_table("stream_smoke");
    let full = format!("{}.{}", namespace, table);

    // 1) Create namespace
    let ns_sql = format!("CREATE NAMESPACE IF NOT EXISTS {}", namespace);
    execute_sql_as_root_via_client(&ns_sql).expect("create namespace should succeed");

    // 2) Create stream table with 30-second TTL
    let create_sql = format!(
        r#"CREATE TABLE {} (
            event_id TEXT NOT NULL,
            event_type TEXT,
            payload TEXT,
            timestamp TIMESTAMP
        ) WITH (TYPE = 'STREAM', TTL_SECONDS = 3)"#,
        full
    );
    execute_sql_as_root_via_client(&create_sql).expect("create stream table should succeed");

    // 3) Subscribe to the stream table
    let query = format!("SELECT * FROM {}", full);
    let mut listener = SubscriptionListener::start(&query).expect("subscription should start");

    // 4) Insert a stream event and expect subscription output
    let ev_val = "smoke_stream_event";
    let mut got_any = false;
    let mut attempt = 0;
    while attempt < 5 && !got_any {
        attempt += 1;
        let event_id = format!("e{}", attempt);
        let ins = format!(
            "INSERT INTO {} (event_id, event_type, payload) VALUES ('{}', 'info', '{}')",
            full, event_id, ev_val
        );
        execute_sql_as_root_via_client(&ins).expect("insert stream event should succeed");

        // After each insert, poll for up to 1s for a subscription line
        let per_attempt_deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while std::time::Instant::now() < per_attempt_deadline {
            match listener.try_read_line(std::time::Duration::from_millis(100)) {
                Ok(Some(line)) => {
                    if !line.trim().is_empty() {
                        got_any = true;
                        break;
                    }
                },
                Ok(None) => break,
                Err(_) => continue,
            }
        }
    }
    assert!(got_any, "expected to receive some subscription output within retry window");

    // Stop subscription
    listener.stop().ok();

    // 5) Verify data is present via regular SELECT immediately after insert
    let select_sql = format!("SELECT * FROM {}", full);
    let select_visible_deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut last_select_output = String::new();
    while std::time::Instant::now() < select_visible_deadline {
        last_select_output =
            execute_sql_as_root_via_client_json(&select_sql).expect("select should succeed");
        if last_select_output.contains(ev_val) {
            break;
        }
    }
    assert!(
        last_select_output.contains(ev_val),
        "expected to find inserted event '{}' in SELECT output within 5s after insert. Output:\n{}",
        ev_val,
        last_select_output
    );

    // 6) Verify data has been evicted via regular SELECT (poll until TTL passes)
    let eviction_deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut select_after_ttl = String::new();
    while std::time::Instant::now() < eviction_deadline {
        select_after_ttl = execute_sql_as_root_via_client_json(&select_sql)
            .expect("select after TTL should succeed");
        if !select_after_ttl.contains(ev_val) {
            break;
        }
        std::thread::yield_now();
    }
    assert!(
        !select_after_ttl.contains(ev_val),
        "expected event '{}' to be evicted within 5 seconds (TTL=3s)",
        ev_val
    );

    // Cleanup
    let _ = execute_sql_as_root_via_client(&format!("DROP NAMESPACE {} CASCADE", namespace));
}
