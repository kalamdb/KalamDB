-- Stress comments, literals, and namespace-local DML in one script.

DROP NAMESPACE IF EXISTS sql_file_case_07 CASCADE;
CREATE NAMESPACE IF NOT EXISTS sql_file_case_07;
USE sql_file_case_07;

/* comment with a semicolon ; should not split the batch */
CREATE TABLE IF NOT EXISTS sql_file_case_07.snippets (
  id BIGINT PRIMARY KEY,
  body TEXT NOT NULL,
  tag TEXT NOT NULL
) WITH (TYPE = 'USER', STORAGE_ID = 'local', FLUSH_POLICY = 'rows:100,interval:60');

DELETE FROM sql_file_case_07.snippets WHERE id IN (7101, 7102);

CREATE TOPIC IF NOT EXISTS snippet_events;
ALTER TOPIC snippet_events ADD SOURCE sql_file_case_07.snippets ON INSERT WITH (payload = 'full');

-- another comment ; with punctuation
INSERT INTO sql_file_case_07.snippets (id, body, tag)
VALUES
  (7101, 'hello; world -- literal text', 'alpha'),
  (7102, '/* not a block comment inside a string */', 'beta');

CONSUME FROM snippet_events LIMIT 10;

DROP TOPIC snippet_events;
DROP NAMESPACE IF EXISTS sql_file_case_07 CASCADE;