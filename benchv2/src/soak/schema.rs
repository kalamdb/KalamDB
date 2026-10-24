//! Checkout schema, topics, and the JavaScript functions the buyers call.

use crate::client::KalamClient;

const SKUS: usize = 32;

pub async fn setup(client: &KalamClient, namespace: &str) -> Result<(), String> {
    client.sql_ok(&format!("CREATE SCHEMA IF NOT EXISTS {namespace}")).await?;
    for sql in table_sql(namespace) {
        client.sql_ok(&sql).await?;
    }
    client.sql_ok(&seed_sql(namespace)).await?;
    for sql in function_sql(namespace) {
        client.sql_ok(&sql).await?;
    }
    for topic in ["order_placed", "order_paid", "order_shipped"] {
        client
            .sql_ok(&format!("CREATE TOPIC IF NOT EXISTS {namespace}.{topic}"))
            .await?;
    }
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    Ok(())
}

pub async fn teardown(client: &KalamClient, namespace: &str) -> Result<(), String> {
    for topic in ["order_placed", "order_paid", "order_shipped"] {
        let _ = client.sql(&format!("DROP TOPIC IF EXISTS {namespace}.{topic}")).await;
    }
    client.sql_ok(&format!("DROP SCHEMA IF EXISTS {namespace}")).await?;
    Ok(())
}

fn table_sql(namespace: &str) -> Vec<String> {
    vec![
        format!(
            "CREATE TABLE {namespace}.catalog (sku TEXT PRIMARY KEY, name TEXT NOT NULL, \
             price_cents INT NOT NULL, stock INT NOT NULL) WITH (TYPE = 'SHARED', FLUSH_POLICY = \
             'interval:60')"
        ),
        format!(
            "CREATE TABLE {namespace}.orders (id BIGINT PRIMARY KEY, customer TEXT NOT NULL, sku \
             TEXT NOT NULL, qty INT NOT NULL, total_cents BIGINT NOT NULL, status TEXT NOT NULL, \
             created_ms BIGINT NOT NULL) WITH (TYPE = 'SHARED')"
        ),
        format!(
            "CREATE TABLE {namespace}.payments (order_id BIGINT PRIMARY KEY, amount_cents BIGINT \
             NOT NULL, fee_cents BIGINT NOT NULL, method TEXT NOT NULL, status TEXT NOT NULL) \
             WITH (TYPE = 'SHARED')"
        ),
        format!(
            "CREATE TABLE {namespace}.shipments (order_id BIGINT PRIMARY KEY, carrier TEXT NOT \
             NULL, tracking TEXT NOT NULL, status TEXT NOT NULL) WITH (TYPE = 'SHARED')"
        ),
        format!(
            "CREATE TABLE {namespace}.inbox (id BIGINT PRIMARY KEY, order_id BIGINT NOT NULL, \
             kind TEXT NOT NULL, body TEXT NOT NULL, created_ms BIGINT NOT NULL) WITH (TYPE = \
             'SHARED')"
        ),
        format!(
            "CREATE TABLE {namespace}.activity (id BIGINT PRIMARY KEY, customer TEXT NOT NULL, \
             kind TEXT NOT NULL, created_ms BIGINT NOT NULL) WITH (TYPE = 'STREAM', TTL_SECONDS = \
             30)"
        ),
    ]
}

fn seed_sql(namespace: &str) -> String {
    let mut values = Vec::with_capacity(SKUS);
    for sku in 0..SKUS {
        let price = 499 + (sku as i64) * 125;
        values.push(format!("('sku_{sku:02}', 'item_{sku:02}', {price}, 100000000)"));
    }
    format!(
        "INSERT INTO {namespace}.catalog (sku, name, price_cents, stock) VALUES {}",
        values.join(", ")
    )
}

fn function_sql(namespace: &str) -> Vec<String> {
    vec![
        quote_fn(namespace),
        place_fn(namespace),
        pay_fn(namespace),
        ship_fn(namespace),
        sweep_fn(namespace),
    ]
}

fn quote_fn(namespace: &str) -> String {
    format!(
        r#"CREATE OR REPLACE FUNCTION {namespace}.quote_cart(customer TEXT, sku TEXT, qty INT, promo TEXT)
RETURNS JSON LANGUAGE JAVASCRIPT AS $$
const qty = input.qty;
const promo = input.promo || "NONE";
if (qty < 1 || qty > 12) return {{ ok: false, reason: "qty" }};
const rows = await ctx.db.query("SELECT price_cents, stock FROM {namespace}.catalog WHERE sku = $1", [input.sku]);
if (!rows || rows.length === 0) return {{ ok: false, reason: "sku" }};
const price = rows[0].price_cents;
const stock = rows[0].stock;
if (stock < qty) return {{ ok: false, reason: "stock" }};
let discount = 0;
const gross = price * qty;
if (promo === "SAVE10") discount = Math.floor(gross * 10 / 100);
else if (promo === "BULK" && qty >= 3) discount = Math.floor(gross * 15 / 100);
const customer = String(input.customer || "");
if (customer.length > 0 && customer.charCodeAt(customer.length - 1) % 2 === 0) {{
  discount += Math.floor(gross * 2 / 100);
}}
const net = gross - discount;
const shipping = net >= 5000 || promo === "FREESHIP" ? 0 : 499;
const tax = Math.floor((net + shipping) * 8 / 100);
return {{
  ok: true,
  sku: input.sku,
  qty: qty,
  unit_cents: price,
  discount_cents: discount,
  shipping_cents: shipping,
  tax_cents: tax,
  total_cents: net + shipping + tax
}};
$$"#
    )
}

fn place_fn(namespace: &str) -> String {
    format!(
        r#"CREATE OR REPLACE FUNCTION {namespace}.place_order(order_id BIGINT, customer TEXT, sku TEXT, qty INT, promo TEXT, created_ms BIGINT)
RETURNS JSON LANGUAGE JAVASCRIPT AS $$
const existing = await ctx.db.query("SELECT status FROM {namespace}.orders WHERE id = $1", [input.order_id]);
if (existing && existing.length > 0) return {{ ok: true, duplicate: true, order_id: input.order_id, status: existing[0].status }};
const quote = await ctx.functions.call("{namespace}.quote_cart", [{{
  customer: input.customer,
  sku: input.sku,
  qty: input.qty,
  promo: input.promo
}}]);
if (!quote || quote.ok !== true) return quote || {{ ok: false, reason: "quote" }};
const taken = await ctx.db.execute(
  "UPDATE {namespace}.catalog SET stock = stock - $2 WHERE sku = $1 AND stock >= $2",
  [input.sku, input.qty]
);
if (!taken) return {{ ok: false, reason: "stock" }};
await ctx.db.execute(
  "INSERT INTO {namespace}.orders (id, customer, sku, qty, total_cents, status, created_ms) VALUES ($1, $2, $3, $4, $5, 'placed', $6)",
  [input.order_id, input.customer, input.sku, input.qty, quote.total_cents, input.created_ms]
);
await ctx.db.execute(
  "INSERT INTO {namespace}.inbox (id, order_id, kind, body, created_ms) VALUES ($1, $2, 'placed', $3, $4)",
  [input.order_id * 10 + 1, input.order_id, String(input.customer) + " placed " + input.sku, input.created_ms]
);
await ctx.db.execute(
  "INSERT INTO {namespace}.activity (id, customer, kind, created_ms) VALUES ($1, $2, 'checkout', $3)",
  [input.order_id, input.customer, input.created_ms]
);
await ctx.topics.publish("{namespace}.order_placed", {{
  orderId: input.order_id,
  token: "ord:" + input.order_id,
  customer: input.customer,
  sku: input.sku,
  total: quote.total_cents
}});
return {{ ok: true, order_id: input.order_id, total_cents: quote.total_cents, status: "placed" }};
$$"#
    )
}

fn pay_fn(namespace: &str) -> String {
    format!(
        r#"CREATE OR REPLACE FUNCTION {namespace}.capture_payment(order_id BIGINT, method TEXT, created_ms BIGINT)
RETURNS JSON LANGUAGE JAVASCRIPT AS $$
const orders = await ctx.db.query(
  "SELECT id, total_cents, status FROM {namespace}.orders WHERE id = $1",
  [input.order_id]
);
if (!orders || orders.length === 0) return {{ ok: false, reason: "missing" }};
const order = orders[0];
if (order.status === "paid" || order.status === "shipped") return {{ ok: true, duplicate: true, status: order.status }};
if (order.status !== "placed") return {{ ok: false, reason: "state" }};
const method = input.method || "card";
let fee = 30 + Math.floor(order.total_cents * 29 / 1000);
if (method === "wallet") fee = Math.floor(order.total_cents * 10 / 1000);
else if (method === "cod") fee = 150;
const updated = await ctx.db.execute(
  "UPDATE {namespace}.orders SET status = 'paid' WHERE id = $1 AND status = 'placed'",
  [input.order_id]
);
if (!updated) return {{ ok: true, duplicate: true }};
await ctx.db.execute(
  "INSERT INTO {namespace}.payments (order_id, amount_cents, fee_cents, method, status) VALUES ($1, $2, $3, $4, 'captured')",
  [input.order_id, order.total_cents, fee, method]
);
await ctx.db.execute(
  "INSERT INTO {namespace}.inbox (id, order_id, kind, body, created_ms) VALUES ($1, $2, 'paid', $3, $4)",
  [input.order_id * 10 + 2, input.order_id, method, input.created_ms]
);
await ctx.topics.publish("{namespace}.order_paid", {{
  orderId: input.order_id,
  token: "ord:" + input.order_id,
  method: method,
  fee: fee
}});
return {{ ok: true, order_id: input.order_id, fee_cents: fee, status: "paid" }};
$$"#
    )
}

fn ship_fn(namespace: &str) -> String {
    format!(
        r#"CREATE OR REPLACE FUNCTION {namespace}.dispatch_shipment(order_id BIGINT, created_ms BIGINT)
RETURNS JSON LANGUAGE JAVASCRIPT AS $$
const orders = await ctx.db.query(
  "SELECT id, status FROM {namespace}.orders WHERE id = $1",
  [input.order_id]
);
if (!orders || orders.length === 0) return {{ ok: false, reason: "missing" }};
const order = orders[0];
if (order.status === "shipped") return {{ ok: true, duplicate: true }};
if (order.status !== "paid") return {{ ok: false, reason: "state" }};
const carriers = ["ups", "fedex", "dhl", "local"];
const carrier = carriers[Number(input.order_id) % carriers.length];
const tracking = carrier + "-" + input.order_id;
const updated = await ctx.db.execute(
  "UPDATE {namespace}.orders SET status = 'shipped' WHERE id = $1 AND status = 'paid'",
  [input.order_id]
);
if (!updated) return {{ ok: true, duplicate: true }};
await ctx.db.execute(
  "INSERT INTO {namespace}.shipments (order_id, carrier, tracking, status) VALUES ($1, $2, $3, 'label_created')",
  [input.order_id, carrier, tracking]
);
await ctx.db.execute(
  "INSERT INTO {namespace}.inbox (id, order_id, kind, body, created_ms) VALUES ($1, $2, 'shipped', $3, $4)",
  [input.order_id * 10 + 3, input.order_id, tracking, input.created_ms]
);
await ctx.topics.publish("{namespace}.order_shipped", {{
  orderId: input.order_id,
  token: "ord:" + input.order_id,
  tracking: tracking
}});
return {{ ok: true, tracking: tracking, status: "shipped" }};
$$"#
    )
}

fn sweep_fn(namespace: &str) -> String {
    format!(
        r#"CREATE OR REPLACE FUNCTION {namespace}.sweep_stale(start_id BIGINT, cutoff_ms BIGINT, end_id BIGINT)
RETURNS JSON LANGUAGE JAVASCRIPT AS $$
const start = Number(input.start_id);
const end = Number(input.end_id);
if (!(end > start)) {{
  return {{ ok: true, restored: 0, next_id: start, caught_up: true }};
}}
const ids = [];
for (let id = start; id < end; id++) ids.push(id);
const list = ids.join(",");
await ctx.db.execute("DELETE FROM {namespace}.orders WHERE id IN (" + list + ")");
await ctx.db.execute("DELETE FROM {namespace}.payments WHERE order_id IN (" + list + ")");
await ctx.db.execute("DELETE FROM {namespace}.shipments WHERE order_id IN (" + list + ")");
const inbox = [];
for (let i = 0; i < ids.length; i++) {{
  const id = ids[i];
  inbox.push(id * 10 + 1, id * 10 + 2, id * 10 + 3);
}}
await ctx.db.execute("DELETE FROM {namespace}.inbox WHERE id IN (" + inbox.join(",") + ")");
return {{ ok: true, restored: ids.length, next_id: end, caught_up: false }};
$$"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn functions_use_host_sql_and_topics() {
        let sql = function_sql("soak_test").join("\n");
        assert!(sql.contains("ctx.db.query"));
        assert!(sql.contains("ctx.db.execute"));
        assert!(sql.contains("ctx.functions.call"));
        assert!(sql.contains("ctx.topics.publish"));
        assert!(sql.contains("soak_test.order_paid"));
        assert!(sql.contains("WHERE id = $1"));
        assert!(sql.contains("WHERE id IN ("));
        assert!(!sql.contains("created_ms < $1"));
        assert!(!sql.contains("CREATE USER TABLE"));
    }
}
