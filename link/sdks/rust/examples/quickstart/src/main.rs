use std::time::Duration;

use kalam_client::{AuthProvider, KalamLinkClient};

fn server_url() -> String {
    std::env::var("KALAMDB_SERVER_URL").unwrap_or_else(|_| "http://localhost:2900".to_string())
}

fn auth() -> AuthProvider {
    let password =
        std::env::var("KALAMDB_ROOT_PASSWORD").unwrap_or_else(|_| "kalamdb123".to_string());
    AuthProvider::system_user_auth(password)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = KalamLinkClient::builder()
        .base_url(server_url())
        .auth(auth())
        .timeout(Duration::from_secs(30))
        .build()?;

    let response = client.execute_query("SELECT CURRENT_USER()", None, None, None).await?;

    println!("status: {:?}", response.status);
    if let Some(result) = response.results.first() {
        if let Some(rows) = &result.rows {
            for row in rows {
                println!("current user row: {row:?}");
            }
        }
    }

    client.disconnect().await;
    Ok(())
}
