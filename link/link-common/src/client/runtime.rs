use std::sync::Arc;

use super::{KalamLinkClient, KalamLinkClientBuilder, QueryUploadFile};
#[cfg(feature = "consumer")]
use crate::consumer::ConsumerBuilder;
use crate::{
    auth::{AuthProvider, ResolvedAuth},
    error::{KalamLinkError, Result},
    event_handlers::EventHandlers,
    models::file_upload::uploads_to_owned,
    models::{FileUpload, LoginResponse, QueryResponse, SubscriptionConfig, SubscriptionInfo},
    query::models::query_param::params_to_json,
    query::models::QueryParam,
    query::UploadProgressCallback,
    subscription::{LiveRowsConfig, LiveRowsSubscription, SubscriptionManager},
    timeouts::KalamLinkTimeouts,
};

impl KalamLinkClient {
    /// Create a new builder for configuring the client
    pub fn builder() -> KalamLinkClientBuilder {
        KalamLinkClientBuilder::new()
    }

    /// Execute a SQL query with optional files, parameters, and namespace context
    ///
    /// # Arguments
    /// * `sql` - The SQL query string
    /// * `files` - Optional file uploads for FILE("name") placeholders
    /// * `params` - Optional query parameters for $1, $2, ... placeholders
    /// * `namespace_id` - Optional namespace for unqualified table names
    ///
    /// # Example
    /// ```rust,no_run
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let client = kalam_client::KalamLinkClient::builder().base_url("http://localhost:3000").build()?;
    /// // Simple query
    /// let result = client.execute_query("SELECT * FROM users", None, None, None).await?;
    ///
    /// // Query with parameters
    /// let params = vec![QueryParam::from("doc1"), QueryParam::from(42_i64)];
    /// let result = client.execute_query("SELECT * FROM users WHERE id = $1", None, Some(params), None).await?;
    ///
    /// // Query in specific namespace
    /// let result = client.execute_query("SELECT * FROM messages", None, None, Some("chat")).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn execute_query(
        &self,
        sql: &str,
        files: Option<Vec<FileUpload>>,
        params: Option<Vec<QueryParam>>,
        namespace_id: Option<&str>,
    ) -> Result<QueryResponse> {
        self.execute_query_with_progress(sql, files, params, namespace_id, None).await
    }

    /// Execute a SQL query with optional files and a progress callback for uploads.
    pub async fn execute_query_with_progress(
        &self,
        sql: &str,
        files: Option<Vec<FileUpload>>,
        params: Option<Vec<QueryParam>>,
        namespace_id: Option<&str>,
        progress: Option<UploadProgressCallback>,
    ) -> Result<QueryResponse> {
        self.query_executor
            .execute_with_progress_ref(
                sql,
                uploads_to_owned(files),
                params_to_json(params),
                namespace_id,
                progress,
            )
            .await
    }

    /// Execute a SQL query with legacy borrowed upload tuples.
    pub async fn execute_query_with_tuples(
        &self,
        sql: &str,
        files: Option<Vec<QueryUploadFile<'_>>>,
        params: Option<Vec<QueryParam>>,
        namespace_id: Option<&str>,
    ) -> Result<QueryResponse> {
        let files = files.map(|items| {
            items
                .into_iter()
                .map(|(placeholder, filename, data, mime)| {
                    let mut upload = FileUpload::new(placeholder, filename, data);
                    if let Some(mime) = mime {
                        upload = upload.with_mime(mime);
                    }
                    upload
                })
                .collect()
        });
        self.execute_query(sql, files, params, namespace_id).await
    }

    /// Execute a SQL query with file uploads (FILE datatype support).
    ///
    /// This method allows inserting/updating rows that contain FILE columns.
    /// Use FILE("name") placeholders in SQL that reference uploaded files.
    #[cfg(feature = "file-uploads")]
    pub async fn execute_with_files(
        &self,
        sql: &str,
        files: Vec<FileUpload>,
        params: Option<Vec<QueryParam>>,
        namespace_id: Option<&str>,
    ) -> Result<QueryResponse> {
        self.execute_query(sql, Some(files), params, namespace_id).await
    }

    /// Execute a SQL query with file uploads and a progress callback.
    #[cfg(feature = "file-uploads")]
    pub async fn execute_with_files_with_progress(
        &self,
        sql: &str,
        files: Vec<FileUpload>,
        params: Option<Vec<QueryParam>>,
        namespace_id: Option<&str>,
        progress: Option<UploadProgressCallback>,
    ) -> Result<QueryResponse> {
        self.execute_query_with_progress(sql, Some(files), params, namespace_id, progress)
            .await
    }

    /// Open a low-level live event stream.
    ///
    /// Live streams are multiplexed over the shared WebSocket connection.
    pub async fn live_events(&self, query: &str) -> Result<SubscriptionManager> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let subscription_id = format!("sub_{}", nanos);
        self.live_events_with_config(SubscriptionConfig::new(subscription_id, query))
            .await
    }

    /// Open a low-level live event stream with advanced configuration.
    ///
    /// When [`ConnectionOptions::ws_lazy_connect`] is `true` (the default)
    /// and no shared connection exists yet, `connect()` is called
    /// automatically before opening the stream.
    pub async fn live_events_with_config(
        &self,
        config: SubscriptionConfig,
    ) -> Result<SubscriptionManager> {
        if self.connection_options.ws_lazy_connect {
            let conn_guard = self.connection.lock().await;
            if conn_guard.is_none() {
                drop(conn_guard);
                self.connect().await?;
            }
        }

        let conn = {
            let conn_guard = self.connection.lock().await;
            conn_guard.clone()
        };

        if let Some(conn) = conn {
            // Send the protocol subscribe command without holding the client
            // connection mutex. The command channel is bounded and can apply
            // backpressure, so awaiting it while locked would serialize or
            // stall unrelated connection operations.
            let (event_rx, result_rx) =
                conn.subscribe_send(config.id.clone(), config.sql, config.options).await?;
            let shared_control = conn.subscription_control();

            // Wait for the server ack without the lock held. Initial snapshot
            // batches continue through the returned subscription stream.
            let (generation, resume_from) = result_rx.await.map_err(|_| {
                KalamLinkError::WebSocketError(
                    "Connection task died before confirming subscribe".to_string(),
                )
            })??;

            return Ok(SubscriptionManager::from_shared(
                config.id,
                event_rx,
                shared_control,
                generation,
                resume_from,
                config.ack_mode,
                &self.timeouts,
            ));
        }

        Err(KalamLinkError::WebSocketError(
            "Not connected. Call connect() before opening live streams.".to_string(),
        ))
    }

    /// Open a SQL query and receive materialized row snapshots.
    pub async fn live(&self, query: &str) -> Result<LiveRowsSubscription> {
        self.live_with_config(
            SubscriptionConfig::new(
                format!(
                    "live_rows_{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos()
                ),
                query,
            ),
            LiveRowsConfig::default(),
        )
        .await
    }

    /// Open materialized live rows with advanced low-level and materialization configuration.
    pub async fn live_with_config(
        &self,
        config: SubscriptionConfig,
        live_rows_config: LiveRowsConfig,
    ) -> Result<LiveRowsSubscription> {
        let subscription = self.live_events_with_config(config).await?;
        Ok(LiveRowsSubscription::new(subscription, live_rows_config))
    }

    /// Establish a shared WebSocket connection.
    ///
    /// After calling this, all subsequent [`live_events()`](Self::live_events)
    /// and [`live()`](Self::live) calls will multiplex over the single connection.
    pub async fn connect(&self) -> Result<()> {
        {
            let conn_guard = self.connection.lock().await;
            if conn_guard.is_some() {
                return Ok(());
            }
        }

        let resolved_auth = match self.fresh_auth().await? {
            AuthProvider::BasicAuth(user, password) => {
                let login_response = self.exchange_login_credentials(&user, &password).await?;
                AuthProvider::jwt_token(login_response.access_token)
            },
            auth => auth,
        };
        self.update_shared_auth(resolved_auth);

        let conn = Arc::new(
            crate::connection::SharedConnection::connect(
                self.base_url.clone(),
                self.shared_resolved_auth.clone(),
                self.timeouts.clone(),
                self.connection_options.clone(),
                self.event_handlers.clone(),
            )
            .await?,
        );

        let mut conn_guard = self.connection.lock().await;
        if conn_guard.is_none() {
            *conn_guard = Some(conn);
        } else {
            drop(conn_guard);
            conn.disconnect().await;
        }
        Ok(())
    }

    /// Disconnect the shared WebSocket connection.
    pub async fn disconnect(&self) {
        let conn = {
            let mut guard = self.connection.lock().await;
            guard.take()
        };
        if let Some(conn) = conn {
            conn.disconnect().await;
        }
    }

    /// Cancel / unsubscribe a subscription by ID on the shared connection.
    pub async fn cancel_subscription(&self, id: &str) -> Result<()> {
        let conn = {
            let guard = self.connection.lock().await;
            guard.clone()
        };
        if let Some(conn) = conn {
            conn.unsubscribe(id).await?;
        }
        Ok(())
    }

    /// Whether the shared connection is currently ready.
    ///
    /// During reconnect with active subscriptions, this stays false until the
    /// subscription set has recovered, not merely until the socket handshake
    /// succeeds.
    pub async fn is_connected(&self) -> bool {
        let guard = self.connection.lock().await;
        guard.as_ref().is_some_and(|conn| conn.is_connected())
    }

    /// List all active subscriptions on the shared connection.
    pub async fn subscriptions(&self) -> Vec<SubscriptionInfo> {
        let conn = {
            let guard = self.connection.lock().await;
            guard.clone()
        };
        match conn.as_ref() {
            Some(conn) => conn.list_subscriptions().await,
            None => Vec::new(),
        }
    }

    /// Get the current event handlers
    pub fn event_handlers(&self) -> &EventHandlers {
        &self.event_handlers
    }

    /// Get the configured timeouts
    pub fn timeouts(&self) -> &KalamLinkTimeouts {
        &self.timeouts
    }

    /// Create a topic consumer builder bound to this client
    #[cfg(feature = "consumer")]
    pub fn consumer(&self) -> ConsumerBuilder {
        ConsumerBuilder::from_client(self.clone())
    }

    #[cfg(feature = "consumer")]
    pub(crate) fn auth(&self) -> &AuthProvider {
        &self.auth
    }

    /// Return the resolved auth source (static or dynamic).
    pub fn resolved_auth(&self) -> &ResolvedAuth {
        &self.resolved_auth
    }

    /// Replace the static authentication credentials at runtime.
    pub fn set_auth(&mut self, auth: AuthProvider) {
        self.auth = auth.clone();
        self.query_executor.set_auth(auth.clone());
        let resolved = ResolvedAuth::Static(auth);
        self.resolved_auth = resolved.clone();
        *self.shared_resolved_auth.write().unwrap() = resolved;
    }

    /// Update the shared authentication source without requiring `&mut self`.
    pub fn update_shared_auth(&self, auth: AuthProvider) {
        self.query_executor.set_auth(auth.clone());
        let resolved = ResolvedAuth::Static(auth);
        *self.shared_resolved_auth.write().unwrap() = resolved;
    }

    /// Resolve fresh credentials from the auth source.
    pub async fn fresh_auth(&self) -> Result<AuthProvider> {
        self.resolved_auth.resolve().await
    }

    pub(crate) fn base_url(&self) -> &str {
        &self.base_url
    }

    pub(crate) fn http_client(&self) -> reqwest::Client {
        self.http_client.clone()
    }

    pub(crate) async fn jwt_for_http_request(&self) -> Result<AuthProvider> {
        match self.fresh_auth().await? {
            AuthProvider::BasicAuth(user, password) => {
                let login = self.exchange_login_credentials(&user, &password).await?;
                Ok(AuthProvider::jwt_token(login.access_token))
            },
            auth @ (AuthProvider::JwtToken(_) | AuthProvider::None) => Ok(auth),
        }
    }

    async fn exchange_login_credentials(
        &self,
        user: &str,
        password: &str,
    ) -> Result<LoginResponse> {
        let url = format!("{}/v1/api/auth/login", self.base_url);
        let body = serde_json::json!({
            "user": user,
            "password": password,
        });

        let response = self.http_client.post(&url).json(&body).send().await?;
        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            return Err(KalamLinkError::AuthenticationError(format!(
                "Login failed during auth exchange ({}): {}",
                status, error_text
            )));
        }

        Ok(response.json::<LoginResponse>().await?)
    }
}
