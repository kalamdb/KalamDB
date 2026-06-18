use kalam_cli::session::auth_options::authenticate_external_token;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
};

#[tokio::test]
async fn oidc_external_token_must_validate_with_kalamdb_before_session_storage() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server_url = format!("http://{}", listener.local_addr().unwrap());
    let (request_tx, request_rx) = oneshot::channel();

    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buffer = [0; 1024];
        let read = socket.read(&mut buffer).await.unwrap();
        let request = String::from_utf8_lossy(&buffer[..read]);
        request_tx.send(request.to_string()).unwrap();

        socket
            .write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 13\r\n\r\ninvalid token")
            .await
            .unwrap();
    });

    let client = reqwest::Client::new();
    let error =
        authenticate_external_token(&client, &server_url, "invalid.external.token".to_string())
            .await
            .expect_err("invalid external token should be rejected");

    assert!(error.to_string().contains("external token was rejected by KalamDB"));
    let request = request_rx.await.unwrap();
    assert!(request.starts_with("GET /v1/api/auth/me HTTP/1.1"));
    assert!(request
        .to_ascii_lowercase()
        .contains("authorization: bearer invalid.external.token"));
    server.await.unwrap();
}
