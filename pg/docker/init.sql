-- KalamDB PostgreSQL Extension — Remote Initialization Script
-- Runs automatically on first container start via docker-entrypoint-initdb.d

-- 1. Install the extension
CREATE EXTENSION IF NOT EXISTS pg_kalam;

-- 2. Create the remote foreign server pointing to the KalamDB compose service.
--    When using docker-compose, 'kalamdb' resolves to the KalamDB container.
--    Use account-login auth so the smoke test can exercise DDL and DML through
--    the same root credentials the compose stack exposes.
CREATE SERVER IF NOT EXISTS kalam_server
	FOREIGN DATA WRAPPER pg_kalam
	OPTIONS (
		host 'kalamdb',
		port '2910',
		auth_mode 'account_login',
		login_user 'root',
		login_password 'kalamdb123'
	);
