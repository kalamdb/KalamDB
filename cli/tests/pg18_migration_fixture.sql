-- PostgreSQL 18 / KalamDB representative migration fixture (T069 / T109).
-- Covers custom enum, composite, domain, sequences/defaults, constraints,
-- scalar indexes, and typed data rows.

CREATE SCHEMA IF NOT EXISTS mig_app;

CREATE TYPE mig_app.order_state AS ENUM ('pending', 'paid', 'shipped');
CREATE TYPE mig_app.shipping_addr AS (city TEXT, postal_code TEXT);
CREATE DOMAIN mig_app.positive_qty AS INT NOT NULL DEFAULT 1 CHECK (VALUE > 0);

CREATE TABLE mig_app.orders (
    id BIGINT PRIMARY KEY,
    customer TEXT NOT NULL,
    qty mig_app.positive_qty,
    amount NUMERIC(10, 2) NOT NULL,
    state mig_app.order_state NOT NULL DEFAULT 'pending',
    addr mig_app.shipping_addr,
    metadata JSONB
) WITH (TYPE = 'SHARED');

CREATE INDEX idx_mig_orders_customer ON mig_app.orders (customer);

INSERT INTO mig_app.orders (id, customer, qty, amount, state, addr, metadata)
VALUES
    (1, 'alice', 2, 49.95, 'paid', ROW('Seattle', '98101')::mig_app.shipping_addr, '{"tier":"gold","Priority":1}'),
    (2, 'bob', DEFAULT, 19.50, DEFAULT, ROW('Austin', '78701')::mig_app.shipping_addr, '{"tier":"silver"}');
