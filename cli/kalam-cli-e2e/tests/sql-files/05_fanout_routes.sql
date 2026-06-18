-- One topic fed by multiple tables and operations.

DROP NAMESPACE IF EXISTS sql_file_case_05 CASCADE;
CREATE NAMESPACE IF NOT EXISTS sql_file_case_05;
USE sql_file_case_05;

CREATE TABLE IF NOT EXISTS orders (
  id BIGINT PRIMARY KEY,
  status TEXT NOT NULL,
  total BIGINT NOT NULL
) WITH (TYPE = 'USER', STORAGE_ID = 'local', FLUSH_POLICY = 'rows:100,interval:60');

CREATE TABLE IF NOT EXISTS order_notes (
  id BIGINT PRIMARY KEY,
  order_id BIGINT NOT NULL,
  note TEXT NOT NULL
) WITH (TYPE = 'USER', STORAGE_ID = 'local', FLUSH_POLICY = 'rows:100,interval:60');

DELETE FROM order_notes WHERE id IN (5201);
DELETE FROM orders WHERE id IN (5101);

CREATE TOPIC IF NOT EXISTS order_activity;
ALTER TOPIC order_activity ADD SOURCE orders ON INSERT WITH (payload = 'full');
ALTER TOPIC order_activity ADD SOURCE orders ON UPDATE WHERE status = 'cancelled';
ALTER TOPIC order_activity ADD SOURCE order_notes ON INSERT WITH (payload = 'full');

INSERT INTO orders (id, status, total) VALUES (5101, 'pending', 9900);
INSERT INTO order_notes (id, order_id, note) VALUES (5201, 5101, 'customer requested gift wrap');
UPDATE orders SET status = 'cancelled' WHERE id = 5101;

CONSUME FROM order_activity LIMIT 20;

DROP TOPIC order_activity;
DROP NAMESPACE IF EXISTS sql_file_case_05 CASCADE;