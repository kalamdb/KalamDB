use std::{sync::atomic::Ordering, time::Duration};

use kalam_client::SubscriptionConfig;
use tokio::time::{sleep, timeout};

use super::helpers::*;
use crate::common::tcp_proxy::TcpDisconnectProxy;

async fn wait_for_row_after_checkpoint(
    sub: &mut kalam_client::SubscriptionManager,
    checkpoint: kalam_client::SeqId,
    expected_ids: &[&str],
    forbidden_ids: &[&str],
    context: &str,
) {
    let mut seen_ids = Vec::<String>::new();
    let mut resumed_seq = Some(checkpoint);

    for _ in 0..60 {
        if expected_ids.iter().all(|expected| seen_ids.iter().any(|seen| seen == expected)) {
            break;
        }

        match timeout(Duration::from_millis(2000), sub.next()).await {
            Ok(Some(Ok(ev))) => {
                collect_ids_and_track_seq(
                    &ev,
                    &mut seen_ids,
                    &mut resumed_seq,
                    Some(checkpoint),
                    context,
                );
            },
            Ok(Some(Err(e))) => panic!("{}: subscription error after impairment: {}", context, e),
            Ok(None) => panic!("{}: subscription ended unexpectedly", context),
            Err(_) => {},
        }
    }

    for expected in expected_ids {
        assert!(
            seen_ids.iter().any(|seen| seen == expected),
            "{}: expected row {} after recovery",
            context,
            expected
        );
    }

    for forbidden in forbidden_ids {
        assert!(
            !seen_ids.iter().any(|seen| seen == forbidden),
            "{}: row {} must not replay after recovery",
            context,
            forbidden
        );
    }
}

/// Traffic can stop flowing while the TCP socket remains open. The client must
/// still detect the dead connection via pong timeout, reconnect, and resume.
#[tokio::test]
async fn test_proxy_blackhole_keeps_socket_open_until_client_times_out() {
    let result = timeout(Duration::from_secs(90), async {
        let writer = match create_test_client() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Skipping test (writer client unavailable): {}", e);
                return;
            },
        };

        let proxy = TcpDisconnectProxy::start(upstream_server_url()).await;
        let (client, connect_count, disconnect_count) =
            match create_test_client_with_events_for_base_url(proxy.base_url()) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("Skipping test (proxy client unavailable): {}", e);
                    proxy.shutdown().await;
                    return;
                },
            };

        let suffix = unique_suffix();
        let table = format!("default.blackhole_timeout_{}", suffix);
        ensure_table(&writer, &table).await;

        client.connect().await.expect("initial connect through proxy");
        let mut sub = client
            .live_events_with_config(SubscriptionConfig::new(
                format!("blackhole-timeout-{}", suffix),
                format!("SELECT id, value FROM {}", table),
            ))
            .await
            .expect("subscribe through proxy");

        let _ = timeout(TEST_TIMEOUT, sub.next()).await;

        writer
            .execute_query(
                &format!("INSERT INTO {} (id, value) VALUES ('baseline', 'ready')", table),
                None,
                None,
                None,
            )
            .await
            .expect("insert baseline row");

        let mut baseline_ids = Vec::<String>::new();
        let mut checkpoint = None;
        for _ in 0..12 {
            if baseline_ids.iter().any(|id| id == "baseline") {
                break;
            }
            match timeout(Duration::from_millis(1200), sub.next()).await {
                Ok(Some(Ok(ev))) => {
                    collect_ids_and_track_seq(
                        &ev,
                        &mut baseline_ids,
                        &mut checkpoint,
                        None,
                        "blackhole baseline",
                    );
                },
                _ => {},
            }
        }
        assert!(baseline_ids.iter().any(|id| id == "baseline"));
        let resume_from = query_max_seq(&writer, &table).await;

        assert!(
            proxy.wait_for_active_connections(1, Duration::from_secs(2)).await,
            "proxy should have at least one active connection before blackhole"
        );

        let disconnects_before = disconnect_count.load(Ordering::SeqCst);
        let expected_connects = connect_count.load(Ordering::SeqCst) + 1;
        proxy.blackhole();

        writer
            .execute_query(
                &format!(
                    "INSERT INTO {} (id, value) VALUES ('gap-blackhole', 'during-outage')",
                    table
                ),
                None,
                None,
                None,
            )
            .await
            .expect("insert gap row during blackhole");

        let mut saw_open_socket_during_blackhole = false;
        for _ in 0..40 {
            if proxy.active_count().await >= 1 {
                saw_open_socket_during_blackhole = true;
            }
            if disconnect_count.load(Ordering::SeqCst) > disconnects_before {
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }

        assert!(
            saw_open_socket_during_blackhole,
            "proxy should keep the TCP socket open while traffic is blackholed"
        );
        assert!(
            disconnect_count.load(Ordering::SeqCst) > disconnects_before,
            "client should detect timeout even though the proxy did not hard-drop the socket"
        );

        proxy.restore_traffic();

        for _ in 0..80 {
            if connect_count.load(Ordering::SeqCst) >= expected_connects
                && client.is_connected().await
            {
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }

        assert!(client.is_connected().await, "client should reconnect after blackhole clears");

        writer
            .execute_query(
                &format!(
                    "INSERT INTO {} (id, value) VALUES ('live-blackhole', 'after-reconnect')",
                    table
                ),
                None,
                None,
                None,
            )
            .await
            .expect("insert live row after reconnect");

        wait_for_row_after_checkpoint(
            &mut sub,
            resume_from,
            &["gap-blackhole", "live-blackhole"],
            &["baseline"],
            "blackhole recovery",
        )
        .await;

        sub.close().await.ok();
        client.disconnect().await;
        proxy.shutdown().await;
    })
    .await;

    assert!(result.is_ok(), "blackhole timeout test timed out");
}

/// High latency below the pong-timeout budget should not trigger false
/// reconnects, and live rows should still flow through the shared connection.
#[tokio::test]
async fn test_proxy_latency_does_not_false_positive_disconnect() {
    let result = timeout(Duration::from_secs(30), async {
        let writer = match create_test_client() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Skipping test (writer client unavailable): {}", e);
                return;
            },
        };

        let proxy = TcpDisconnectProxy::start(upstream_server_url()).await;
        let (client, _connect_count, disconnect_count) =
            match create_test_client_with_events_for_base_url(proxy.base_url()) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("Skipping test (proxy client unavailable): {}", e);
                    proxy.shutdown().await;
                    return;
                },
            };

        let suffix = unique_suffix();
        let table = format!("default.latency_tolerant_{}", suffix);
        ensure_table(&writer, &table).await;

        client.connect().await.expect("initial connect through proxy");
        let mut sub = client
            .live_events_with_config(SubscriptionConfig::new(
                format!("latency-tolerant-{}", suffix),
                format!("SELECT id, value FROM {}", table),
            ))
            .await
            .expect("subscribe through proxy");

        let _ = timeout(TEST_TIMEOUT, sub.next()).await;

        let disconnects_before = disconnect_count.load(Ordering::SeqCst);
        proxy.set_latency(Duration::from_millis(300));
        sleep(Duration::from_secs(4)).await;

        assert!(
            client.is_connected().await,
            "client should stay connected under moderate latency"
        );
        assert_eq!(
            disconnect_count.load(Ordering::SeqCst),
            disconnects_before,
            "moderate latency should not trigger a disconnect"
        );

        writer
            .execute_query(
                &format!("INSERT INTO {} (id, value) VALUES ('slow-network', 'still-live')", table),
                None,
                None,
                None,
            )
            .await
            .expect("insert row under latency");

        let mut ids = Vec::<String>::new();
        let mut seq = None;
        for _ in 0..12 {
            if ids.iter().any(|id| id == "slow-network") {
                break;
            }
            match timeout(Duration::from_millis(1500), sub.next()).await {
                Ok(Some(Ok(ev))) => {
                    collect_ids_and_track_seq(&ev, &mut ids, &mut seq, None, "moderate latency");
                },
                _ => {},
            }
        }

        assert!(
            ids.iter().any(|id| id == "slow-network"),
            "live row should still arrive under moderate latency"
        );

        proxy.clear_latency();
        sub.close().await.ok();
        client.disconnect().await;
        proxy.shutdown().await;
    })
    .await;

    assert!(result.is_ok(), "moderate latency test timed out");
}

/// Simulate packet loss at the TCP level as retransmission-like chunk stalls.
/// The client should reconnect and resume once the stalls are removed.
#[tokio::test]
async fn test_proxy_packet_loss_style_stalls_resume_without_replay() {
    let result = timeout(Duration::from_secs(60), async {
        let writer = match create_test_client() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Skipping test (writer client unavailable): {}", e);
                return;
            },
        };

        let proxy = TcpDisconnectProxy::start(upstream_server_url()).await;
        let (client, connect_count, disconnect_count) =
            match create_test_client_with_events_for_base_url(proxy.base_url()) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("Skipping test (proxy client unavailable): {}", e);
                    proxy.shutdown().await;
                    return;
                },
            };

        let suffix = unique_suffix();
        let table = format!("default.lossy_stalls_{}", suffix);
        ensure_table(&writer, &table).await;

        client.connect().await.expect("initial connect through proxy");
        let mut sub = client
            .live_events_with_config(SubscriptionConfig::new(
                format!("lossy-stalls-{}", suffix),
                format!("SELECT id, value FROM {}", table),
            ))
            .await
            .expect("subscribe through proxy");

        let _ = timeout(TEST_TIMEOUT, sub.next()).await;

        writer
            .execute_query(
                &format!("INSERT INTO {} (id, value) VALUES ('baseline-lossy', 'ready')", table),
                None,
                None,
                None,
            )
            .await
            .expect("insert baseline row");

        let mut baseline_ids = Vec::<String>::new();
        let mut checkpoint = None;
        for _ in 0..12 {
            if baseline_ids.iter().any(|id| id == "baseline-lossy") {
                break;
            }
            match timeout(Duration::from_millis(1200), sub.next()).await {
                Ok(Some(Ok(ev))) => {
                    collect_ids_and_track_seq(
                        &ev,
                        &mut baseline_ids,
                        &mut checkpoint,
                        None,
                        "lossy baseline",
                    );
                },
                _ => {},
            }
        }
        assert!(baseline_ids.iter().any(|id| id == "baseline-lossy"));
        let resume_from = query_max_seq(&writer, &table).await;

        let disconnects_before = disconnect_count.load(Ordering::SeqCst);
        let expected_connects = connect_count.load(Ordering::SeqCst) + 1;
        // This needs to exceed the 2s pong timeout used by reconnect_test_timeouts()
        // or the transport impairment will look like mere latency instead of a dead link.
        proxy.set_chunk_stall_pattern(1, Duration::from_millis(2500));

        writer
            .execute_query(
                &format!("INSERT INTO {} (id, value) VALUES ('gap-lossy', 'during-stall')", table),
                None,
                None,
                None,
            )
            .await
            .expect("insert gap row during stall pattern");

        for _ in 0..80 {
            if disconnect_count.load(Ordering::SeqCst) > disconnects_before {
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }

        assert!(
            disconnect_count.load(Ordering::SeqCst) > disconnects_before,
            "retransmission-style stalls should eventually force a reconnect"
        );

        proxy.clear_chunk_stall_pattern();

        for _ in 0..120 {
            if connect_count.load(Ordering::SeqCst) >= expected_connects
                && client.is_connected().await
            {
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }

        assert!(
            client.is_connected().await,
            "client should reconnect after stall pattern clears"
        );

        writer
            .execute_query(
                &format!(
                    "INSERT INTO {} (id, value) VALUES ('live-lossy', 'after-reconnect')",
                    table
                ),
                None,
                None,
                None,
            )
            .await
            .expect("insert live row after stall pattern");

        wait_for_row_after_checkpoint(
            &mut sub,
            resume_from,
            &["gap-lossy", "live-lossy"],
            &["baseline-lossy"],
            "lossy recovery",
        )
        .await;

        sub.close().await.ok();
        client.disconnect().await;
        proxy.shutdown().await;
    })
    .await;

    assert!(result.is_ok(), "packet-loss-style stall test timed out");
}

/// Very small write slices fragment WebSocket frames at the transport boundary.
/// The client should parse the stream normally and avoid spurious reconnects.
#[tokio::test]
#[ntest::timeout(30000)]
async fn test_tokio_netem_fragmented_writes_preserve_live_stream() {
    let result = timeout(Duration::from_secs(45), async {
        let writer = match create_test_client() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Skipping test (writer client unavailable): {}", e);
                return;
            },
        };

        let proxy = TcpDisconnectProxy::start(upstream_server_url()).await;
        let (client, _connect_count, disconnect_count) =
            match create_test_client_with_events_for_base_url(proxy.base_url()) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("Skipping test (proxy client unavailable): {}", e);
                    proxy.shutdown().await;
                    return;
                },
            };

        let suffix = unique_suffix();
        let table = format!("default.netem_sliced_{}", suffix);
        ensure_table(&writer, &table).await;

        client.connect().await.expect("initial connect through proxy");
        let mut sub = client
            .live_events_with_config(SubscriptionConfig::new(
                format!("netem-sliced-{}", suffix),
                format!("SELECT id, value FROM {}", table),
            ))
            .await
            .expect("subscribe through proxy");

        let _ = timeout(TEST_TIMEOUT, sub.next()).await;
        let disconnects_before = disconnect_count.load(Ordering::SeqCst);
        proxy.set_netem_write_slice_size(3);

        writer
            .execute_query(
                &format!(
                    "INSERT INTO {} (id, value) VALUES ('fragmented', '{}')",
                    table,
                    "x".repeat(512)
                ),
                None,
                None,
                None,
            )
            .await
            .expect("insert row through fragmented transport");

        let mut ids = Vec::<String>::new();
        let mut seq = None;
        for _ in 0..30 {
            if ids.iter().any(|id| id == "fragmented") {
                break;
            }
            match timeout(Duration::from_millis(1000), sub.next()).await {
                Ok(Some(Ok(ev))) => {
                    collect_ids_and_track_seq(&ev, &mut ids, &mut seq, None, "netem slicing");
                },
                Ok(Some(Err(e))) => panic!("netem slicing subscription error: {}", e),
                Ok(None) => panic!("netem slicing subscription ended unexpectedly"),
                Err(_) => {},
            }
        }

        assert!(
            ids.iter().any(|id| id == "fragmented"),
            "live row should arrive when netem fragments writes"
        );
        assert_eq!(
            disconnect_count.load(Ordering::SeqCst),
            disconnects_before,
            "write fragmentation should not cause a reconnect"
        );

        proxy.clear_netem_write_slice_size();
        sub.close().await.ok();
        client.disconnect().await;
        proxy.shutdown().await;
    })
    .await;

    assert!(result.is_ok(), "netem fragmented write test timed out");
}

/// A severe bandwidth collapse should look like a dead link to keepalive. Once
/// the throttle is removed, reconnect resume must deliver only rows after the
/// checkpoint.
#[tokio::test]
#[ntest::timeout(30000)]
async fn test_tokio_netem_bandwidth_collapse_forces_resume_without_replay() {
    let result = timeout(Duration::from_secs(75), async {
        let writer = match create_test_client() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Skipping test (writer client unavailable): {}", e);
                return;
            },
        };

        let proxy = TcpDisconnectProxy::start(upstream_server_url()).await;
        let (client, connect_count, disconnect_count) =
            match create_test_client_with_events_for_base_url(proxy.base_url()) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("Skipping test (proxy client unavailable): {}", e);
                    proxy.shutdown().await;
                    return;
                },
            };

        let suffix = unique_suffix();
        let table = format!("default.netem_throttle_{}", suffix);
        ensure_table(&writer, &table).await;

        client.connect().await.expect("initial connect through proxy");
        let mut sub = client
            .live_events_with_config(SubscriptionConfig::new(
                format!("netem-throttle-{}", suffix),
                format!("SELECT id, value FROM {}", table),
            ))
            .await
            .expect("subscribe through proxy");

        let _ = timeout(TEST_TIMEOUT, sub.next()).await;
        writer
            .execute_query(
                &format!("INSERT INTO {} (id, value) VALUES ('baseline-throttle', 'ready')", table),
                None,
                None,
                None,
            )
            .await
            .expect("insert baseline row");

        let mut baseline_ids = Vec::<String>::new();
        let mut checkpoint = None;
        for _ in 0..12 {
            if baseline_ids.iter().any(|id| id == "baseline-throttle") {
                break;
            }
            match timeout(Duration::from_millis(1200), sub.next()).await {
                Ok(Some(Ok(ev))) => {
                    collect_ids_and_track_seq(
                        &ev,
                        &mut baseline_ids,
                        &mut checkpoint,
                        None,
                        "netem throttle baseline",
                    );
                },
                _ => {},
            }
        }
        assert!(baseline_ids.iter().any(|id| id == "baseline-throttle"));
        let resume_from = query_max_seq(&writer, &table).await;

        let disconnects_before = disconnect_count.load(Ordering::SeqCst);
        let expected_connects = connect_count.load(Ordering::SeqCst) + 1;
        proxy.set_netem_write_rate(8);

        writer
            .execute_query(
                &format!(
                    "INSERT INTO {} (id, value) VALUES ('gap-throttle', 'during-collapse')",
                    table
                ),
                None,
                None,
                None,
            )
            .await
            .expect("insert gap row during bandwidth collapse");

        for _ in 0..80 {
            if disconnect_count.load(Ordering::SeqCst) > disconnects_before {
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }
        assert!(
            disconnect_count.load(Ordering::SeqCst) > disconnects_before,
            "severe netem throttle should force a reconnect"
        );

        proxy.clear_netem_write_rate();
        wait_for_reconnect(&client, &connect_count, expected_connects, "netem throttle").await;

        writer
            .execute_query(
                &format!(
                    "INSERT INTO {} (id, value) VALUES ('live-throttle', 'after-reconnect')",
                    table
                ),
                None,
                None,
                None,
            )
            .await
            .expect("insert live row after throttle clears");

        wait_for_row_after_checkpoint(
            &mut sub,
            resume_from,
            &["gap-throttle", "live-throttle"],
            &["baseline-throttle"],
            "netem throttle recovery",
        )
        .await;

        sub.close().await.ok();
        client.disconnect().await;
        proxy.shutdown().await;
    })
    .await;

    assert!(result.is_ok(), "netem bandwidth collapse test timed out");
}

/// tokio-netem can fail the transport from inside the I/O adapter instead of
/// aborting the proxy task. The client should treat it as a normal disconnect.
#[tokio::test]
#[ntest::timeout(15000)]
async fn test_tokio_netem_forced_transport_termination_recovers() {
    let result = timeout(Duration::from_secs(75), async {
        let writer = match create_test_client() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Skipping test (writer client unavailable): {}", e);
                return;
            },
        };

        let proxy = TcpDisconnectProxy::start(upstream_server_url()).await;
        let (client, connect_count, disconnect_count) =
            match create_test_client_with_events_for_base_url(proxy.base_url()) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("Skipping test (proxy client unavailable): {}", e);
                    proxy.shutdown().await;
                    return;
                },
            };

        let suffix = unique_suffix();
        let table = format!("default.netem_terminator_{}", suffix);
        ensure_table(&writer, &table).await;

        client.connect().await.expect("initial connect through proxy");
        let mut sub = client
            .live_events_with_config(SubscriptionConfig::new(
                format!("netem-terminator-{}", suffix),
                format!("SELECT id, value FROM {}", table),
            ))
            .await
            .expect("subscribe through proxy");

        let _ = timeout(TEST_TIMEOUT, sub.next()).await;
        writer
            .execute_query(
                &format!("INSERT INTO {} (id, value) VALUES ('baseline-term', 'ready')", table),
                None,
                None,
                None,
            )
            .await
            .expect("insert baseline row");

        let mut baseline_ids = Vec::<String>::new();
        let mut checkpoint = None;
        for _ in 0..12 {
            if baseline_ids.iter().any(|id| id == "baseline-term") {
                break;
            }
            match timeout(Duration::from_millis(1200), sub.next()).await {
                Ok(Some(Ok(ev))) => {
                    collect_ids_and_track_seq(
                        &ev,
                        &mut baseline_ids,
                        &mut checkpoint,
                        None,
                        "netem termination baseline",
                    );
                },
                _ => {},
            }
        }
        assert!(baseline_ids.iter().any(|id| id == "baseline-term"));
        let resume_from = query_max_seq(&writer, &table).await;

        let disconnects_before = disconnect_count.load(Ordering::SeqCst);
        let expected_connects = connect_count.load(Ordering::SeqCst) + 1;
        proxy.set_netem_termination_probability(1.0);

        writer
            .execute_query(
                &format!(
                    "INSERT INTO {} (id, value) VALUES ('gap-term', 'during-termination')",
                    table
                ),
                None,
                None,
                None,
            )
            .await
            .expect("insert gap row during transport termination");

        for _ in 0..80 {
            if disconnect_count.load(Ordering::SeqCst) > disconnects_before {
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }
        assert!(
            disconnect_count.load(Ordering::SeqCst) > disconnects_before,
            "netem terminator should force a disconnect"
        );

        proxy.clear_netem_termination_probability();
        wait_for_reconnect(&client, &connect_count, expected_connects, "netem terminator").await;

        writer
            .execute_query(
                &format!(
                    "INSERT INTO {} (id, value) VALUES ('live-term', 'after-reconnect')",
                    table
                ),
                None,
                None,
                None,
            )
            .await
            .expect("insert live row after terminator clears");

        wait_for_row_after_checkpoint(
            &mut sub,
            resume_from,
            &["gap-term", "live-term"],
            &["baseline-term"],
            "netem terminator recovery",
        )
        .await;

        sub.close().await.ok();
        client.disconnect().await;
        proxy.shutdown().await;
    })
    .await;

    assert!(result.is_ok(), "netem forced termination test timed out");
}
