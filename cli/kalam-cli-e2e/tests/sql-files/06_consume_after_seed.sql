-- Simple consume flow after namespace-local inserts.

DROP NAMESPACE IF EXISTS sql_file_case_06 CASCADE;
CREATE NAMESPACE IF NOT EXISTS sql_file_case_06;
USE sql_file_case_06;

CREATE TABLE IF NOT EXISTS sql_file_case_06.notifications (
  id BIGINT PRIMARY KEY,
  channel TEXT NOT NULL,
  body TEXT NOT NULL
) WITH (TYPE = 'USER', STORAGE_ID = 'local', FLUSH_POLICY = 'rows:100,interval:60');

DELETE FROM sql_file_case_06.notifications WHERE id IN (6101, 6102, 6103);

CREATE TOPIC IF NOT EXISTS notification_stream;
ALTER TOPIC notification_stream ADD SOURCE sql_file_case_06.notifications ON INSERT WITH (payload = 'full');

INSERT INTO sql_file_case_06.notifications (id, channel, body)
VALUES
  (6101, 'email', 'welcome'),
  (6102, 'push', 'order shipped'),
  (6103, 'sms', 'otp generated');

CONSUME FROM notification_stream GROUP 'fixture-consumers-06' FROM EARLIEST LIMIT 10;

DROP TOPIC notification_stream;
DROP NAMESPACE IF EXISTS sql_file_case_06 CASCADE;