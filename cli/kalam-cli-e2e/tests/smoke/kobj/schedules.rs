//! CREATE / ALTER / DROP SCHEDULE smoke: configuration, failed calls, and real wakeups.

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use chrono::{Timelike, Utc};
use serde_json::Value;

use crate::{common::*, kobj_helpers::*};

fn require_server() {
    assert!(is_server_running(), "schedule smoke requires a running server");
}

fn create_js_procedure(ns: &str, name: &str, params: &str, body: &str) {
    exec(&format!(
        "CREATE OR REPLACE PROCEDURE {ns}.{name}({params}) LANGUAGE JAVASCRIPT AS $$\n{body}\n$$"
    ));
}

fn ok_tick_body(ns: &str, schedule: &str, hits: &str) -> String {
    format!(
        "if (ctx.source.kind !== 'schedule' || ctx.source.scheduleId !== '{ns}.{schedule}' || \
         !ctx.source.runId || typeof ctx.source.scheduledAt !== 'number' || ctx.http !== null) \
         throw new Error('invalid schedule origin'); return ctx.db.execute(\"INSERT INTO {hits} \
         (run_id, kind) VALUES ('\" + ctx.source.runId + \"', '\" + ctx.source.kind + \"')\");"
    )
}

fn create_hits_table(ns: &str) -> String {
    let hits = format!("{ns}.schedule_hits");
    exec(&format!(
        "CREATE TABLE {hits} (run_id TEXT PRIMARY KEY, kind TEXT) WITH (TYPE = 'SHARED')"
    ));
    grant_public_shared_table_access(&hits);
    ready(&hits);
    hits
}

fn schedule_rows(id: &str) -> Vec<HashMap<String, Value>> {
    query_rows(&format!(
        "SELECT schedule_id, enabled, cron, interval_ms, timezone, next_run_at, run_count, \
         skip_count, last_error, last_finished_at, running_until FROM system.schedules WHERE \
         schedule_id = '{id}'"
    ))
}

fn wait_schedule(
    id: &str,
    timeout: Duration,
    predicate: impl Fn(&HashMap<String, Value>) -> bool,
) -> HashMap<String, Value> {
    let started = Instant::now();
    loop {
        let rows = schedule_rows(id);
        assert_eq!(rows.len(), 1, "schedule {id} missing: {rows:?}");
        if predicate(&rows[0]) {
            return rows[0].clone();
        }
        assert!(started.elapsed() < timeout, "timed out waiting for {id}: {:?}", rows[0]);
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn disable_and_drop_namespace(ns: &str, schedules: &[&str]) {
    for name in schedules {
        let _ = execute_sql_as_root_via_client(&format!("ALTER SCHEDULE {ns}.{name} DISABLE"));
        let _ = execute_sql_as_root_via_client(&format!("DROP SCHEDULE IF EXISTS {ns}.{name}"));
    }
    exec(&format!("DROP NAMESPACE {ns} CASCADE"));
}

#[test]
#[ntest::timeout(180000)]
fn smoke_schedule_01_rejects_missing_procedure() {
    require_server();
    let (ns, _cleanup) = setup_ephemeral_namespace("kobj_sched_miss");
    let err = exec_err(&format!(
        "CREATE SCHEDULE {ns}.clock INTERVAL '1 second' EXECUTE PROCEDURE {ns}.does_not_exist()"
    ));
    assert!(
        err.to_ascii_lowercase().contains("procedure") && err.contains("does_not_exist"),
        "missing procedure must fail CREATE SCHEDULE: {err}"
    );
    assert!(schedule_rows(&format!("{ns}.clock")).is_empty());
    exec(&format!("DROP NAMESPACE {ns} CASCADE"));
}

#[test]
#[ntest::timeout(30000)]
fn smoke_schedule_02_rejects_parameterized_procedure() {
    require_server();
    let (ns, _cleanup) = setup_ephemeral_namespace("kobj_sched_args");
    create_js_procedure(&ns, "needs_arg", "x INT", "return input.x;");
    let err = exec_err(&format!(
        "CREATE SCHEDULE {ns}.clock INTERVAL '1 second' EXECUTE PROCEDURE {ns}.needs_arg()"
    ));
    assert!(
        err.to_ascii_lowercase().contains("no parameters"),
        "parameterized procedure must be rejected: {err}"
    );
    exec(&format!("DROP NAMESPACE {ns} CASCADE"));
}

#[test]
#[ntest::timeout(30000)]
fn smoke_schedule_03_rejects_missing_namespace() {
    require_server();
    let (ns, _cleanup) = setup_ephemeral_namespace("kobj_sched_ns");
    create_js_procedure(&ns, "tick", "", "return 1;");
    let ghost = format!("ghost_{}", &ns[ns.len().saturating_sub(8)..]);
    let err = exec_err(&format!(
        "CREATE SCHEDULE {ghost}.clock INTERVAL '1 second' EXECUTE PROCEDURE {ns}.tick()"
    ));
    assert!(
        err.to_ascii_lowercase().contains("namespace") && err.contains(&ghost),
        "missing schedule namespace must fail: {err}"
    );
    exec(&format!("DROP NAMESPACE {ns} CASCADE"));
}

#[test]
#[ntest::timeout(30000)]
fn smoke_schedule_04_rejects_invalid_cron_interval_timezone_and_principal() {
    require_server();
    let (ns, _cleanup) = setup_ephemeral_namespace("kobj_sched_cfg");
    create_js_procedure(&ns, "tick", "", "return 1;");
    let cases = [
        (
            format!("CREATE SCHEDULE {ns}.six CRON '* * * * * *' EXECUTE PROCEDURE {ns}.tick()"),
            "five fields",
        ),
        (
            format!("CREATE SCHEDULE {ns}.zero INTERVAL '0 seconds' EXECUTE PROCEDURE {ns}.tick()"),
            "interval",
        ),
        (
            format!(
                "CREATE SCHEDULE {ns}.tz CRON '* * * * *' TIME ZONE 'Not/AZone' EXECUTE PROCEDURE \
                 {ns}.tick()"
            ),
            "time zone",
        ),
        (
            format!(
                "CREATE SCHEDULE {ns}.misfire INTERVAL '1 second' EXECUTE PROCEDURE {ns}.tick() \
                 WITH (misfire = 'catch_up')"
            ),
            "skip",
        ),
        (
            format!(
                "CREATE SCHEDULE {ns}.who INTERVAL '1 second' EXECUTE PROCEDURE {ns}.tick() WITH \
                 (principal = 'no_such_schedule_user')"
            ),
            "principal",
        ),
        (
            format!(
                "CREATE SCHEDULE {ns}.payload INTERVAL '1 second' EXECUTE PROCEDURE \
                 {ns}.tick(PAYLOAD)"
            ),
            "expected",
        ),
    ];
    for (sql, needle) in cases {
        let err = exec_err(&sql);
        assert!(
            err.to_ascii_lowercase().contains(needle),
            "expected '{needle}' in configuration error for {sql}: {err}"
        );
    }
    exec(&format!("DROP NAMESPACE {ns} CASCADE"));
}

#[test]
#[ntest::timeout(30000)]
fn smoke_schedule_05_rejects_duplicate_name() {
    require_server();
    let (ns, _cleanup) = setup_ephemeral_namespace("kobj_sched_dup");
    create_js_procedure(&ns, "tick", "", "return 1;");
    exec(&format!(
        "CREATE SCHEDULE {ns}.clock INTERVAL '1 hour' EXECUTE PROCEDURE {ns}.tick()"
    ));
    let err = exec_err(&format!(
        "CREATE SCHEDULE {ns}.clock INTERVAL '1 hour' EXECUTE PROCEDURE {ns}.tick()"
    ));
    assert!(
        err.to_ascii_lowercase().contains("already exists"),
        "duplicate schedule must fail: {err}"
    );
    disable_and_drop_namespace(&ns, &["clock"]);
}

#[test]
#[ntest::timeout(30000)]
fn smoke_schedule_06_interval_wakes_and_runs_procedure() {
    require_server();
    let (ns, _cleanup) = setup_ephemeral_namespace("kobj_sched_wake");
    let hits = create_hits_table(&ns);
    create_js_procedure(&ns, "tick", "", &ok_tick_body(&ns, "clock", &hits));
    exec(&format!(
        "CREATE SCHEDULE {ns}.clock INTERVAL '1 second' EXECUTE PROCEDURE {ns}.tick() WITH \
         (principal = 'system')"
    ));
    let row = wait_schedule(&format!("{ns}.clock"), Duration::from_secs(15), |row| {
        cell_i64(row, "run_count").unwrap_or(0) >= 1 && !is_null(row, "last_finished_at")
    });
    assert!(is_null(&row, "last_error"), "successful wakeup must clear last_error: {row:?}");
    let hit_count = count_sql(&format!("SELECT COUNT(*) FROM {hits}"));
    assert!(hit_count >= 1, "interval wakeup must INSERT from the procedure: {hit_count}");
    let kinds = query_rows(&format!("SELECT kind FROM {hits}"));
    assert!(
        kinds.iter().any(|row| cell_str(row, "kind").as_deref() == Some("schedule")),
        "hit rows must record ctx.source.kind=schedule: {kinds:?}"
    );
    let logs = query_rows(&format!(
        "SELECT origin, outcome, schedule_id FROM system.procedure_logs WHERE schedule_id = \
         '{ns}.clock' AND origin = 'schedule' AND outcome = 'ok'"
    ));
    assert!(!logs.is_empty(), "procedure_logs must record origin=schedule: {logs:?}");
    let expected_schedule = format!("{ns}.clock");
    assert!(
        logs.iter()
            .any(|row| cell_str(row, "schedule_id").as_deref() == Some(expected_schedule.as_str())),
        "scheduled invoke must stamp schedule_id: {logs:?}"
    );
    disable_and_drop_namespace(&ns, &["clock"]);
}

#[test]
#[ntest::timeout(45000)]
fn smoke_schedule_07_records_failed_and_missing_function_calls() {
    require_server();
    let (ns, _cleanup) = setup_ephemeral_namespace("kobj_sched_fail");
    let marker = format!("schedule-boom-{ns}");
    create_js_procedure(&ns, "boom", "", &format!("throw new Error('{marker}');"));
    create_js_procedure(
        &ns,
        "missing_call",
        "",
        &format!("return ctx.functions.call('{ns}.does_not_exist');"),
    );
    exec(&format!(
        "CREATE SCHEDULE {ns}.boom INTERVAL '1 second' EXECUTE PROCEDURE {ns}.boom() WITH \
         (principal = 'system')"
    ));
    exec(&format!(
        "CREATE SCHEDULE {ns}.missing INTERVAL '1 second' EXECUTE PROCEDURE {ns}.missing_call() \
         WITH (principal = 'system')"
    ));

    let boom = wait_schedule(&format!("{ns}.boom"), Duration::from_secs(15), |row| {
        cell_i64(row, "run_count").unwrap_or(0) >= 1 && !is_null(row, "last_error")
    });
    let boom_error = cell_str(&boom, "last_error").unwrap_or_default();
    assert!(
        boom_error.contains(&marker),
        "thrown procedure must surface on the schedule: {boom:?}"
    );
    assert_eq!(cell_bool(&boom, "enabled"), Some(true));

    let missing = wait_schedule(&format!("{ns}.missing"), Duration::from_secs(15), |row| {
        cell_i64(row, "run_count").unwrap_or(0) >= 1 && !is_null(row, "last_error")
    });
    let missing_error = cell_str(&missing, "last_error").unwrap_or_default().to_ascii_lowercase();
    assert!(
        missing_error.contains("does_not_exist")
            || missing_error.contains("not found")
            || missing_error.contains("missing"),
        "missing function call must fail the scheduled run: {missing:?}"
    );

    let errors = query_rows(&format!(
        "SELECT origin, outcome, procedure_id, schedule_id FROM system.procedure_logs WHERE \
         origin = 'schedule' AND outcome = 'error' AND (schedule_id = '{ns}.boom' OR schedule_id \
         = '{ns}.missing')"
    ));
    assert!(
        errors
            .iter()
            .any(|row| cell_str(row, "procedure_id").unwrap_or_default().contains("boom")),
        "failed scheduled call must be in procedure_logs: {errors:?}"
    );
    disable_and_drop_namespace(&ns, &["boom", "missing"]);
}

#[test]
#[ntest::timeout(30000)]
fn smoke_schedule_08_cron_next_run_is_nine_utc() {
    require_server();
    let (ns, _cleanup) = setup_ephemeral_namespace("kobj_sched_nine");
    create_js_procedure(&ns, "tick", "", "return 1;");
    exec(&format!(
        "CREATE SCHEDULE {ns}.daily CRON '0 9 * * *' TIME ZONE 'UTC' EXECUTE PROCEDURE {ns}.tick()"
    ));
    exec(&format!(
        "CREATE SCHEDULE {ns}.ny CRON '0 9 * * *' TIME ZONE 'America/New_York' EXECUTE PROCEDURE \
         {ns}.tick()"
    ));
    let daily = &schedule_rows(&format!("{ns}.daily"))[0];
    assert_eq!(cell_str(daily, "cron").as_deref(), Some("0 9 * * *"));
    assert_eq!(cell_str(daily, "timezone").as_deref(), Some("UTC"));
    let next = cell_i64(daily, "next_run_at").expect("next_run_at");
    let next_utc = chrono::DateTime::from_timestamp_millis(next).expect("next_run timestamp");
    assert_eq!(next_utc.hour(), 9, "UTC cron 0 9 * * * must land on 09:00: {next_utc}");
    assert_eq!(next_utc.minute(), 0);
    assert_eq!(next_utc.second(), 0);
    assert!(next > Utc::now().timestamp_millis() - 2_000);

    let ny = &schedule_rows(&format!("{ns}.ny"))[0];
    assert_eq!(cell_str(ny, "timezone").as_deref(), Some("America/New_York"));
    let ny_next = cell_i64(ny, "next_run_at").expect("ny next_run_at");
    assert_ne!(ny_next, next, "America/New_York 09:00 must differ from UTC 09:00");
    disable_and_drop_namespace(&ns, &["daily", "ny"]);
}

#[test]
#[ntest::timeout(180000)]
fn smoke_schedule_09_cron_every_minute_wakes_and_runs() {
    require_server();
    let (ns, _cleanup) = setup_ephemeral_namespace("kobj_sched_cron");
    let hits = create_hits_table(&ns);
    create_js_procedure(&ns, "tick", "", &ok_tick_body(&ns, "minutely", &hits));
    exec(&format!(
        "CREATE SCHEDULE {ns}.minutely CRON '* * * * *' TIME ZONE 'UTC' EXECUTE PROCEDURE \
         {ns}.tick() WITH (principal = 'system')"
    ));
    let created = &schedule_rows(&format!("{ns}.minutely"))[0];
    assert_eq!(cell_str(created, "cron").as_deref(), Some("* * * * *"));
    let next = cell_i64(created, "next_run_at").expect("next_run_at");
    let next_utc = chrono::DateTime::from_timestamp_millis(next).expect("next_run timestamp");
    assert_eq!(next_utc.second(), 0, "five-field cron fires on the minute: {next_utc}");

    let row = wait_schedule(&format!("{ns}.minutely"), Duration::from_secs(90), |row| {
        cell_i64(row, "run_count").unwrap_or(0) >= 1 && !is_null(row, "last_finished_at")
    });
    assert!(is_null(&row, "last_error"), "cron wakeup should succeed: {row:?}");
    assert!(
        count_sql(&format!("SELECT COUNT(*) FROM {hits}")) >= 1,
        "cron wakeup must execute the procedure"
    );
    let logs = query_rows(&format!(
        "SELECT origin, outcome, schedule_id FROM system.procedure_logs WHERE schedule_id = \
         '{ns}.minutely' AND origin = 'schedule' AND outcome = 'ok'"
    ));
    assert!(!logs.is_empty(), "cron firing must log origin=schedule: {logs:?}");
    disable_and_drop_namespace(&ns, &["minutely"]);
}

#[test]
#[ntest::timeout(45000)]
fn smoke_schedule_10_disable_skips_overlap_and_blocks_drop_procedure() {
    require_server();
    let (ns, _cleanup) = setup_ephemeral_namespace("kobj_sched_life");
    create_js_procedure(
        &ns,
        "tick",
        "",
        &format!(
            "if (ctx.source.kind !== 'schedule' || ctx.http !== null) throw new Error('bad \
             origin'); return ctx.sleep(1500).then(function () {{ return 'scheduled'; }});"
        ),
    );
    exec(&format!(
        "CREATE SCHEDULE {ns}.clock INTERVAL '1 second' EXECUTE PROCEDURE {ns}.tick() WITH \
         (principal = 'system', overlap = 'skip', misfire = 'skip')"
    ));
    let started = Instant::now();
    wait_schedule(&format!("{ns}.clock"), Duration::from_secs(15), |row| {
        !is_null(row, "last_finished_at")
    });
    exec(&format!("ALTER SCHEDULE {ns}.clock DISABLE"));
    let disabled = &schedule_rows(&format!("{ns}.clock"))[0];
    assert_eq!(cell_bool(disabled, "enabled"), Some(false));
    assert!(
        cell_i64(disabled, "skip_count").unwrap_or(0) > 0,
        "overlapping 1s interval must skip: {disabled:?}"
    );
    wait_schedule(&format!("{ns}.clock"), Duration::from_secs(20), |row| {
        is_null(row, "running_until")
    });
    let run_count = cell_i64(&schedule_rows(&format!("{ns}.clock"))[0], "run_count").unwrap_or(0);
    std::thread::sleep(Duration::from_millis(2500));
    let later = &schedule_rows(&format!("{ns}.clock"))[0];
    assert_eq!(
        cell_i64(later, "run_count").unwrap_or(0),
        run_count,
        "disabled schedule must not keep waking: {later:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(40));

    let drop_proc = exec_err(&format!("DROP PROCEDURE {ns}.tick"));
    assert!(
        drop_proc.to_ascii_lowercase().contains("schedule"),
        "DROP PROCEDURE must require dropping dependent schedules: {drop_proc}"
    );
    exec(&format!("DROP SCHEDULE IF EXISTS {ns}.clock"));
    exec(&format!("DROP SCHEDULE IF EXISTS {ns}.clock"));
    assert!(schedule_rows(&format!("{ns}.clock")).is_empty());
    exec(&format!("DROP PROCEDURE {ns}.tick"));
    exec(&format!("DROP NAMESPACE {ns} CASCADE"));
}
