use kalamdb_commons::NamespaceId;
use kalamdb_dialect::classifier::SqlStatement;

#[test]
fn schedule_ddl_accepts_postgresql_shaped_statements() {
    for sql in [
        "CREATE SCHEDULE reports.daily CRON '0 9 * * *' TIME ZONE 'UTC' EXECUTE PROCEDURE \
         reports.summary() WITH (principal = 'system', overlap = 'skip', misfire = 'skip')",
        "CREATE SCHEDULE reports.quick INTERVAL '10 seconds' EXECUTE PROCEDURE reports.summary()",
        "ALTER SCHEDULE reports.daily DISABLE",
        "ALTER SCHEDULE reports.daily ENABLE",
        "DROP SCHEDULE IF EXISTS reports.daily",
    ] {
        assert!(
            SqlStatement::classify_and_parse(
                sql,
                &NamespaceId::new("default"),
                kalamdb_commons::Role::System
            )
            .is_ok(),
            "{sql}"
        );
    }
}

#[test]
fn schedule_ddl_rejects_mysql_and_unsupported_policies() {
    for sql in [
        "CREATE EVENT reports.daily ON SCHEDULE EVERY 1 DAY DO CALL reports.summary()",
        "CREATE SCHEDULE `daily` INTERVAL '1 second' EXECUTE PROCEDURE reports.summary()",
        "CREATE SCHEDULE daily CRON '* * * * * *' EXECUTE PROCEDURE summary()",
        "CREATE SCHEDULE daily INTERVAL '0 seconds' EXECUTE PROCEDURE summary()",
        "CREATE SCHEDULE daily INTERVAL '1 second' EXECUTE PROCEDURE summary(PAYLOAD)",
        "CREATE SCHEDULE daily CRON '* * * * *' TIME ZONE 'Fake/Zone' EXECUTE PROCEDURE summary()",
        "CREATE SCHEDULE daily INTERVAL '1 second' EXECUTE PROCEDURE summary() WITH (misfire = \
         'catch_up')",
        "CREATE SCHEDULE daily INTERVAL '1 second' EXECUTE PROCEDURE summary() WITH (overlap = \
         'skip', overlap = 'skip')",
        "ALTER SCHEDULE daily ENABLE garbage",
        "DROP SCHEDULE daily garbage",
    ] {
        assert!(
            SqlStatement::classify_and_parse(
                sql,
                &NamespaceId::new("default"),
                kalamdb_commons::Role::System
            )
            .is_err(),
            "{sql}"
        );
    }
}

#[test]
fn quoted_schedule_identifiers_preserve_case_and_escaped_quotes() {
    use kalamdb_dialect::ddl::CreateScheduleStatement;
    let stmt = CreateScheduleStatement::parse(
        "CREATE /* clock */ SCHEDULE reports.\"Daily\" INTERVAL '1 hour' EXECUTE PROCEDURE \
         reports.\"Summary\"()",
        &NamespaceId::new("default"),
    )
    .unwrap();
    assert_eq!(stmt.schedule_id.as_str(), "reports.Daily");
    assert_eq!(stmt.routine_id.as_str(), "reports.Summary");
}

#[test]
fn schedule_ddl_requires_admin() {
    assert!(SqlStatement::classify_and_parse(
        "DROP SCHEDULE reports.daily",
        &NamespaceId::new("default"),
        kalamdb_commons::Role::User
    )
    .is_err());
}
