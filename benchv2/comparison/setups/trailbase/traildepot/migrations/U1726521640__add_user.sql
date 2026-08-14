INSERT INTO _user (email, password_hash, verified, admin) VALUES ('admin@localhost', (hash_password('secret')), TRUE, TRUE);

-- NOTE: We use a hard-coded user id just to make the setup slightly simpler
-- with a hard-coded id in the benchmark drivers.
INSERT INTO
  _user (id, email, password_hash, verified, admin)
VALUES
  (X'0191FC99879D7352BB86BB1366074704', 'user@localhost', (hash_password('secret')), TRUE, FALSE);
