CREATE NAMESPACE IF NOT EXISTS demo;

DROP TABLE IF EXISTS demo.activity_feed;

CREATE USER TABLE IF NOT EXISTS demo.activity_feed (
    id BIGINT PRIMARY KEY DEFAULT SNOWFLAKE_ID(),
    service TEXT NOT NULL,
    level TEXT NOT NULL,
    actor TEXT NOT NULL,
    message TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

EXECUTE AS USER 'demo-user' (INSERT INTO demo.activity_feed (service, level, actor, message) VALUES ('api', 'ok', 'system', 'Subscriptions are live for this browser session'));
EXECUTE AS USER 'demo-user' (INSERT INTO demo.activity_feed (service, level, actor, message) VALUES ('payments', 'warn', 'ops-bot', 'Retry queue grew above the morning baseline'));
EXECUTE AS USER 'demo-user' (INSERT INTO demo.activity_feed (service, level, actor, message) VALUES ('search', 'critical', 'pager', 'Cold shard promoted and traffic recovered'));