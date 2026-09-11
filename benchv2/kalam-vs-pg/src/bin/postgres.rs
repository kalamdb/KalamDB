//! PostgreSQL side of the pgwire comparison.

use std::env;

use anyhow::Context;
use kalam_vs_pg::{
    connect_client, connect_pool, postgres_setup_sql, run_comparison, run_setup, LIMIT,
    POSTGRES_DATABASE, POSTGRES_PASSWORD, POSTGRES_USER,
};
use tokio_postgres::Config;

fn wire_url() -> String {
    env::var("POSTGRES_URL").unwrap_or_else(|_| {
        format!(
            "postgres://{POSTGRES_USER}:{POSTGRES_PASSWORD}@127.0.0.1:25433/{POSTGRES_DATABASE}"
        )
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let wire = wire_url();
    let config: Config = wire.parse().context("POSTGRES_URL")?;
    let setup_client = connect_client(&config).await?;
    run_setup(&setup_client, &postgres_setup_sql()).await?;
    drop(setup_client);

    let pool = connect_pool(&config, LIMIT).await?;
    println!("target=postgres");
    println!("wire={wire}");
    println!("protocol=pgwire prepared statements");
    println!("pool={LIMIT}");
    println!("mode=native heap + btree PK; synchronous_commit=off (script default)");
    println!("sql=parameterized {}", kalam_vs_pg::INSERT_SQL);
    run_comparison(pool).await
}
