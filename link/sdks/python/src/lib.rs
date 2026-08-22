use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyRuntimeError};
use pyo3::prelude::*;
use pyo3::types::PyAny;
use std::sync::Arc;
use tokio::sync::Mutex;

use kalam_client::query::models::query_param::from_json_value;
use kalam_client::{
    AuthProvider, FileDownload, FileRef, FileUpload, KalamLinkClient, KalamLinkError,
    LiveRowsConfig, LiveRowsEvent, LiveRowsSubscription, QueryParam, SeqId, SubscriptionConfig,
    SubscriptionManager, SubscriptionOptions,
};
use kalam_client::{AutoOffsetReset, TopicConsumer};

create_exception!(kalamdb, KalamError, PyException, "Base class for all KalamDB SDK errors.");
create_exception!(
    kalamdb,
    KalamConnectionError,
    KalamError,
    "Network, WebSocket, or timeout error."
);
create_exception!(
    kalamdb,
    KalamAuthError,
    KalamError,
    "Authentication failure (bad credentials, expired token)."
);
create_exception!(kalamdb, KalamServerError, KalamError, "Server returned an error response.");
create_exception!(kalamdb, KalamConfigError, KalamError, "Invalid client configuration.");

/// Returns true if the error is a TOKEN_EXPIRED server response that we
/// should react to by re-authenticating and retrying once.
fn is_token_expired_error(err: &KalamLinkError) -> bool {
    // The server reports expired tokens via ServerError with a specific code
    // embedded in the error message. We could parse the body JSON, but the
    // error's Display already includes the status/message and TOKEN_EXPIRED
    // as a substring is a reliable signal.
    matches!(err, KalamLinkError::ServerError { .. }) && err.to_string().contains("TOKEN_EXPIRED")
}

/// Ensure the client behind `state` is authenticated. No-op if already so.
///
/// Holds the mutex guard for the whole login, which serializes concurrent
/// first-login attempts — only one caller actually hits the auth endpoint,
/// the rest wait then see `authenticated = true` and return.
async fn ensure_authenticated_locked(state: &mut ClientState) -> PyResult<()> {
    if state.authenticated {
        return Ok(());
    }
    match &state.original_auth {
        AuthProvider::BasicAuth(username, password) => {
            let login = state.client.login(username, password).await.map_err(to_py_err)?;
            let token = login.access_token.clone();
            state.client.set_auth(AuthProvider::jwt_token(token.clone()));
            state.jwt = Some(token);
        },
        AuthProvider::JwtToken(_) | AuthProvider::None => {
            // Already set on builder / no auth needed.
        },
    }
    state.authenticated = true;
    Ok(())
}

/// Run an `execute_query` with auto-refresh on TOKEN_EXPIRED.
///
/// On the first attempt, ensures the client is authenticated. If the call
/// returns TOKEN_EXPIRED, marks the client as needing re-auth, re-logins,
/// and retries exactly once. All other errors propagate immediately.
async fn execute_with_auth_retry(
    state: &Arc<Mutex<ClientState>>,
    sql: &str,
    files: Option<Vec<FileUpload>>,
    params: Option<Vec<QueryParam>>,
) -> PyResult<kalam_client::QueryResponse> {
    for attempt in 0..2u32 {
        let mut guard = state.lock().await;
        ensure_authenticated_locked(&mut guard).await?;

        let result = guard.client.execute_query(sql, files.clone(), params.clone(), None).await;

        match result {
            Ok(response) => return Ok(response),
            Err(e) if attempt == 0 && is_token_expired_error(&e) => {
                guard.authenticated = false;
                guard.jwt = None;
                continue;
            },
            Err(e) => return Err(to_py_err(e)),
        }
    }
    unreachable!("retry loop always returns");
}

fn new_subscription_id(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{prefix}_{nanos}")
}

fn py_seq_id(value: Option<Bound<'_, PyAny>>) -> PyResult<Option<SeqId>> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_none() {
        return Ok(None);
    }
    if let Ok(raw) = value.extract::<i64>() {
        return Ok(Some(SeqId::from_i64(raw)));
    }
    let raw: String = value.extract()?;
    SeqId::from_string(&raw).map(Some).map_err(KalamConfigError::new_err)
}

fn live_subscription_config(
    sql: String,
    prefix: &str,
    batch_size: Option<usize>,
    last_rows: Option<u32>,
    from: Option<SeqId>,
    auto_fetch_batches: Option<bool>,
) -> SubscriptionConfig {
    let mut options = SubscriptionOptions::new();
    if let Some(value) = batch_size {
        options = options.with_batch_size(value);
    }
    if let Some(value) = last_rows {
        options = options.with_last_rows(value);
    }
    if let Some(value) = from {
        options = options.with_from(value);
    }
    if let Some(value) = auto_fetch_batches {
        options = options.with_auto_fetch_batches(value);
    }

    let mut config = SubscriptionConfig::new(new_subscription_id(prefix), sql);
    config.options = Some(options);
    config
}

fn table_live_sql(table: &str) -> PyResult<String> {
    for part in table.split('.') {
        if part.is_empty() || !part.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
            return Err(KalamConfigError::new_err(format!(
                "invalid table name '{table}'; expected namespace.table identifiers"
            )));
        }
    }
    Ok(format!("SELECT * FROM {table}"))
}

fn checkpoint_dict(py: Python<'_>, subscription_id: &str, seq_id: SeqId) -> PyResult<Py<PyAny>> {
    let dict = pyo3::types::PyDict::new(py);
    dict.set_item("subscription_id", subscription_id)?;
    dict.set_item("last_seq_id", seq_id.as_i64().to_string())?;
    Ok(dict.unbind().into())
}

fn call_checkpoint(
    callback: Option<&Py<PyAny>>,
    subscription_id: &str,
    seq_id: Option<SeqId>,
) -> PyResult<()> {
    let Some(seq_id) = seq_id else {
        return Ok(());
    };
    let Some(callback) = callback else {
        return Ok(());
    };
    Python::attach(|py| {
        let checkpoint = checkpoint_dict(py, subscription_id, seq_id)?;
        callback.bind(py).call1((checkpoint,))?;
        Ok(())
    })
}

fn change_event_checkpoint(event: &kalam_client::ChangeEvent) -> Option<(&str, SeqId)> {
    match event {
        kalam_client::ChangeEvent::Ack {
            subscription_id,
            batch_control,
            ..
        } => batch_control.last_seq_id.map(|seq| (subscription_id.as_str(), seq)),
        kalam_client::ChangeEvent::InitialDataBatch {
            subscription_id,
            rows,
            batch_control,
        } => batch_control
            .last_seq_id
            .or_else(|| kalam_client::seq_tracking::extract_max_seq(rows))
            .map(|seq| (subscription_id.as_str(), seq)),
        kalam_client::ChangeEvent::Insert {
            subscription_id,
            rows,
        } => kalam_client::seq_tracking::extract_max_seq(rows)
            .map(|seq| (subscription_id.as_str(), seq)),
        kalam_client::ChangeEvent::Update {
            subscription_id,
            rows,
            old_rows,
        } => kalam_client::seq_tracking::extract_max_seq(rows)
            .or_else(|| kalam_client::seq_tracking::extract_max_seq(old_rows))
            .map(|seq| (subscription_id.as_str(), seq)),
        kalam_client::ChangeEvent::Delete {
            subscription_id,
            old_rows,
        } => kalam_client::seq_tracking::extract_max_seq(old_rows)
            .map(|seq| (subscription_id.as_str(), seq)),
        kalam_client::ChangeEvent::Error { .. } | kalam_client::ChangeEvent::Unknown { .. } => None,
    }
}

fn call_error(callback: Option<&Py<PyAny>>, message: &serde_json::Value) -> PyResult<()> {
    let Some(callback) = callback else {
        return Ok(());
    };
    Python::attach(|py| {
        let py_message = serde_json_value_to_py(py, message)?;
        callback.bind(py).call1((py_message,))?;
        Ok(())
    })
}

/// Convert a KalamLinkError to the right Python exception class.
fn to_py_err(err: KalamLinkError) -> PyErr {
    let message = err.to_string();
    match err {
        KalamLinkError::NetworkError(_)
        | KalamLinkError::WebSocketError(_)
        | KalamLinkError::TimeoutError(_) => KalamConnectionError::new_err(message),
        KalamLinkError::AuthenticationError(_) => KalamAuthError::new_err(message),
        KalamLinkError::ServerError { .. } | KalamLinkError::SetupRequired(_) => {
            KalamServerError::new_err(message)
        },
        KalamLinkError::ConfigurationError(_) => KalamConfigError::new_err(message),
        KalamLinkError::SerializationError(_) | KalamLinkError::Cancelled => {
            PyRuntimeError::new_err(message)
        },
    }
}

/// Typed FILE column reference (mirrors the Rust [`FileRef`] model).
#[pyclass(from_py_object)]
#[derive(Clone)]
struct PyFileRef {
    inner: FileRef,
}

#[pymethods]
impl PyFileRef {
    #[new]
    #[pyo3(signature = (id, sub, name, size, mime, sha256, shard=None))]
    fn new(
        id: String,
        sub: String,
        name: String,
        size: u64,
        mime: String,
        sha256: String,
        shard: Option<u32>,
    ) -> Self {
        Self {
            inner: FileRef {
                id,
                sub,
                name,
                size,
                mime,
                sha256,
                shard,
            },
        }
    }

    #[staticmethod]
    fn from_value(value: Bound<'_, PyAny>) -> PyResult<Option<Self>> {
        let json_value = py_to_json_value(&value)?;
        Ok(FileRef::from_json_value(&json_value).map(|inner| Self { inner }))
    }

    #[getter]
    fn id(&self) -> &str {
        &self.inner.id
    }

    #[getter]
    fn sub(&self) -> &str {
        &self.inner.sub
    }

    #[getter]
    fn name(&self) -> &str {
        &self.inner.name
    }

    #[getter]
    fn size(&self) -> u64 {
        self.inner.size
    }

    #[getter]
    fn mime(&self) -> &str {
        &self.inner.mime
    }

    #[getter]
    fn sha256(&self) -> &str {
        &self.inner.sha256
    }

    #[getter]
    fn shard(&self) -> Option<u32> {
        self.inner.shard
    }

    fn is_image(&self) -> bool {
        self.inner.is_image()
    }

    fn is_pdf(&self) -> bool {
        self.inner.is_pdf()
    }

    fn format_size(&self) -> String {
        self.inner.format_size()
    }

    fn download_url(&self, base_url: &str, namespace: &str, table: &str) -> String {
        self.inner.download_url(base_url, namespace, table)
    }

    fn relative_url(&self, namespace: &str, table: &str) -> String {
        self.inner.relative_url(namespace, table)
    }

    fn __repr__(&self) -> String {
        format!("FileRef({}, {}, {})", self.inner.id, self.inner.name, self.inner.mime)
    }
}

impl From<FileRef> for PyFileRef {
    fn from(inner: FileRef) -> Self {
        Self { inner }
    }
}

/// Multipart upload payload for SQL `FILE("placeholder")` expressions.
#[pyclass(from_py_object)]
#[derive(Clone)]
struct PyFileUpload {
    placeholder: String,
    filename: String,
    data: Vec<u8>,
    mime: Option<String>,
}

#[pymethods]
impl PyFileUpload {
    #[new]
    #[pyo3(signature = (placeholder, filename, data, mime=None))]
    fn new(placeholder: String, filename: String, data: Vec<u8>, mime: Option<String>) -> Self {
        Self {
            placeholder,
            filename,
            data,
            mime,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "FileUpload(placeholder={:?}, filename={:?}, bytes={})",
            self.placeholder,
            self.filename,
            self.data.len()
        )
    }
}

impl PyFileUpload {
    fn into_upload(&self) -> FileUpload {
        let mut upload =
            FileUpload::new(self.placeholder.clone(), self.filename.clone(), self.data.clone());
        if let Some(mime) = &self.mime {
            upload = upload.with_mime(mime.clone());
        }
        upload
    }
}

/// Bytes and HTTP metadata from a file download response.
#[pyclass]
struct PyFileDownload {
    #[pyo3(get)]
    bytes: Vec<u8>,
    #[pyo3(get)]
    content_type: Option<String>,
    #[pyo3(get)]
    content_disposition: Option<String>,
}

impl From<FileDownload> for PyFileDownload {
    fn from(download: FileDownload) -> Self {
        Self {
            bytes: download.bytes,
            content_type: download.content_type,
            content_disposition: download.content_disposition,
        }
    }
}

/// Authentication helper — mirrors Auth.basic() / Auth.jwt() from the TS SDK.
#[pyclass]
struct Auth;

#[pymethods]
impl Auth {
    /// Create basic auth credentials.
    ///
    /// Example:
    ///     auth = Auth.basic("admin", "password")
    #[staticmethod]
    fn basic(username: &str, password: &str) -> Py<PyAny> {
        Python::attach(|py| {
            let dict = pyo3::types::PyDict::new(py);
            dict.set_item("type", "basic").unwrap();
            dict.set_item("username", username).unwrap();
            dict.set_item("password", password).unwrap();
            dict.into_any().unbind()
        })
    }

    /// Create JWT auth credentials.
    ///
    /// Example:
    ///     auth = Auth.jwt("eyJhbG...")
    #[staticmethod]
    fn jwt(token: &str) -> Py<PyAny> {
        Python::attach(|py| {
            let dict = pyo3::types::PyDict::new(py);
            dict.set_item("type", "jwt").unwrap();
            dict.set_item("token", token).unwrap();
            dict.into_any().unbind()
        })
    }
}

/// Internal state — holds the underlying Rust client plus auth state for
/// lazy login and on-expiry refresh.
struct ClientState {
    client: KalamLinkClient,
    /// The auth credentials the user provided. Kept so we can re-run login()
    /// when the JWT expires.
    original_auth: AuthProvider,
    /// Current JWT (None if not yet authenticated or anonymous).
    jwt: Option<String>,
    /// True once we've successfully authenticated the underlying client.
    authenticated: bool,
}

/// KalamDB Python client.
///
/// Construction is synchronous and does NO network I/O — the first query
/// (or an explicit `await client.connect()`) triggers login. If the JWT
/// expires during a long-running session, the SDK will silently re-login
/// using the original credentials on the next request.
///
/// Example:
///     client = KalamClient("http://localhost:2900", Auth.basic("admin", "pass"))
///     result = await client.query("SELECT * FROM app.users")
///     await client.disconnect()
#[pyclass]
struct KalamClient {
    state: Arc<Mutex<ClientState>>,
    url: String,
}

#[pymethods]
impl KalamClient {
    /// Create a new KalamDB client.
    ///
    /// This does NOT contact the server — the first query (or an explicit
    /// `await client.connect()`) triggers the login handshake.
    ///
    /// Args:
    ///     url: The KalamDB server URL (e.g. "http://localhost:2900")
    ///     auth: Auth credentials from Auth.basic() or Auth.jwt()
    ///     options: Optional dict of client settings:
    ///         - timeout_seconds (float): per-request timeout (default 30)
    ///         - max_retries (int): max HTTP retry attempts (default 3)
    #[new]
    #[pyo3(signature = (url, auth, options=None))]
    fn new(
        url: &str,
        auth: &Bound<'_, pyo3::types::PyDict>,
        options: Option<Bound<'_, pyo3::types::PyDict>>,
    ) -> PyResult<Self> {
        let auth_type: String = auth
            .get_item("type")?
            .ok_or_else(|| KalamConfigError::new_err("auth dict missing 'type' key"))?
            .extract()?;

        let provider = match auth_type.as_str() {
            "basic" => {
                let username: String = auth
                    .get_item("username")?
                    .ok_or_else(|| KalamConfigError::new_err("auth dict missing 'username'"))?
                    .extract()?;
                let password: String = auth
                    .get_item("password")?
                    .ok_or_else(|| KalamConfigError::new_err("auth dict missing 'password'"))?
                    .extract()?;
                AuthProvider::basic_auth(username, password)
            },
            "jwt" => {
                let token: String = auth
                    .get_item("token")?
                    .ok_or_else(|| KalamConfigError::new_err("auth dict missing 'token'"))?
                    .extract()?;
                AuthProvider::jwt_token(token)
            },
            other => {
                return Err(KalamConfigError::new_err(format!(
                    "Unknown auth type: '{other}'. Use Auth.basic() or Auth.jwt()"
                )));
            },
        };

        let url_owned = url.to_string();

        // Optional settings
        let mut timeout_seconds: Option<f64> = None;
        let mut max_retries: Option<u32> = None;
        if let Some(opts) = options {
            if let Some(v) = opts.get_item("timeout_seconds")? {
                timeout_seconds = Some(v.extract()?);
            }
            if let Some(v) = opts.get_item("max_retries")? {
                max_retries = Some(v.extract()?);
            }
        }

        // Build the Rust client synchronously — this does NOT touch the network.
        let mut builder = KalamLinkClient::builder().base_url(&url_owned).auth(provider.clone());
        if let Some(secs) = timeout_seconds {
            builder = builder.timeout(std::time::Duration::from_secs_f64(secs));
        }
        if let Some(retries) = max_retries {
            builder = builder.max_retries(retries);
        }
        let client = builder.build().map_err(to_py_err)?;

        // For JWT auth, the token is already set on the builder — consider it
        // authenticated. For Basic, defer login to the first request. For None,
        // no auth is needed.
        let (authenticated, jwt) = match &provider {
            AuthProvider::JwtToken(token) => (true, Some(token.clone())),
            AuthProvider::None => (true, None),
            AuthProvider::BasicAuth(_, _) => (false, None),
        };

        Ok(Self {
            state: Arc::new(Mutex::new(ClientState {
                client,
                original_auth: provider,
                jwt,
                authenticated,
            })),
            url: url_owned,
        })
    }

    /// TEST-ONLY: simulate an expired/invalidated JWT.
    ///
    /// Clears the authenticated flag and cached JWT so the next operation has
    /// to re-login. Used by the test suite to verify auth-refresh behaviour.
    /// Not part of the stable public API — don't rely on this in application code.
    fn _test_invalidate_auth<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let state = self.state.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut guard = state.lock().await;
            guard.authenticated = false;
            guard.jwt = None;
            Ok(Python::attach(|py| py.None()))
        })
    }

    /// Explicitly connect and authenticate now, rather than lazily on first query.
    ///
    /// Optional — the client auto-connects on first use. Call this if you want
    /// to fail fast on bad credentials or an unreachable server at startup.
    fn connect<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let state = self.state.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut guard = state.lock().await;
            ensure_authenticated_locked(&mut guard).await?;
            Ok(Python::attach(|py| py.None()))
        })
    }

    /// Execute a SQL query and return the full response.
    ///
    /// Args:
    ///     sql: SQL query string, optionally with $1, $2, ... placeholders
    ///     params: Optional list of values to bind to the placeholders
    ///
    /// Example:
    ///     # Plain query
    ///     result = await client.query("SELECT * FROM app.users LIMIT 10")
    ///
    ///     # Parameterized query (recommended for user input)
    ///     result = await client.query(
    ///         "SELECT * FROM app.users WHERE id = $1 AND active = $2",
    ///         [42, True]
    ///     )
    #[pyo3(signature = (sql, params=None))]
    fn query<'py>(
        &self,
        py: Python<'py>,
        sql: String,
        params: Option<Bound<'py, pyo3::types::PyList>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let state = self.state.clone();
        let params = py_params_to_query_params(params)?;

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let response = execute_with_auth_retry(&state, &sql, None, params).await?;
            Python::attach(|py| serialize_to_py(py, &response))
        })
    }

    /// Execute SQL and return rows from the first result set.
    ///
    /// Convenience method that extracts just the rows as a list of dicts.
    ///
    /// Example:
    ///     rows = await client.query_rows(
    ///         "SELECT id, name FROM app.users WHERE active = $1",
    ///         [True]
    ///     )
    ///     for row in rows:
    ///         print(row["name"])
    #[pyo3(signature = (sql, params=None))]
    fn query_rows<'py>(
        &self,
        py: Python<'py>,
        sql: String,
        params: Option<Bound<'py, pyo3::types::PyList>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let state = self.state.clone();
        let params = py_params_to_query_params(params)?;

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let response = execute_with_auth_retry(&state, &sql, None, params).await?;

            Python::attach(|py| {
                let rows = pyo3::types::PyList::empty(py);

                if let Some(result) = response.results.first() {
                    if let Some(named_rows) = &result.named_rows {
                        // Server-provided named rows (preferred when available)
                        for row in named_rows {
                            let dict = pyo3::types::PyDict::new(py);
                            for (key, value) in row {
                                let py_val = serde_json_value_to_py(py, value)?;
                                dict.set_item(key, py_val)?;
                            }
                            rows.append(dict)?;
                        }
                    } else if let Some(positional_rows) = &result.rows {
                        // Build dicts from schema + positional rows
                        let column_names: Vec<&str> =
                            result.schema.iter().map(|f| f.name.as_str()).collect();

                        for row in positional_rows {
                            let dict = pyo3::types::PyDict::new(py);
                            for (i, value) in row.iter().enumerate() {
                                if let Some(name) = column_names.get(i) {
                                    let py_val = serde_json_value_to_py(py, value)?;
                                    dict.set_item(*name, py_val)?;
                                }
                            }
                            rows.append(dict)?;
                        }
                    }
                }

                Ok(rows.unbind())
            })
        })
    }

    /// Execute a SQL query with file uploads.
    ///
    /// Use FILE("placeholder") in your SQL to reference uploaded files.
    ///
    /// Args:
    ///     sql: SQL query containing FILE("name") placeholders
    ///     files: Dict mapping placeholder name to (filename, bytes) or (filename, bytes, mime_type)
    ///     params: Optional list of values for $1, $2, ... placeholders
    ///
    /// Example:
    ///     with open("avatar.png", "rb") as f:
    ///         data = f.read()
    ///     await client.query_with_files(
    ///         "INSERT INTO app.users (name, avatar) VALUES ($1, FILE('avatar'))",
    ///         {"avatar": ("avatar.png", data, "image/png")},
    ///         ["alice"],
    ///     )
    #[pyo3(signature = (sql, files, params=None))]
    fn query_with_files<'py>(
        &self,
        py: Python<'py>,
        sql: String,
        files: Bound<'py, pyo3::types::PyDict>,
        params: Option<Bound<'py, pyo3::types::PyList>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let state = self.state.clone();
        let params = py_params_to_query_params(params)?;

        let mut owned_files: Vec<FileUpload> = Vec::new();
        for (key, value) in files.iter() {
            let placeholder: String = key.extract()?;
            if let Ok(upload) = value.extract::<PyRef<PyFileUpload>>() {
                owned_files.push(upload.into_upload());
                continue;
            }

            let tuple = value.cast::<pyo3::types::PyTuple>().map_err(|_| {
                KalamConfigError::new_err(format!(
                    "files['{placeholder}'] must be a FileUpload or a tuple of (filename, bytes[, mime])"
                ))
            })?;

            if tuple.len() < 2 || tuple.len() > 3 {
                return Err(KalamConfigError::new_err(format!(
                    "files['{placeholder}'] must be a 2- or 3-tuple"
                )));
            }

            let filename: String = tuple.get_item(0)?.extract()?;
            let data: Vec<u8> = tuple.get_item(1)?.extract()?;
            let mime: Option<String> = if tuple.len() == 3 {
                Some(tuple.get_item(2)?.extract()?)
            } else {
                None
            };

            let mut upload = FileUpload::new(placeholder.clone(), filename, data);
            if let Some(mime) = mime {
                upload = upload.with_mime(mime);
            }
            owned_files.push(upload);
        }

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let response = execute_with_auth_retry(&state, &sql, Some(owned_files), params).await?;
            Python::attach(|py| serialize_to_py(py, &response))
        })
    }

    /// Download bytes for a [`FileRef`] returned from a query row.
    #[pyo3(signature = (file_ref, namespace, table, target_user_id=None))]
    fn download_file<'py>(
        &self,
        py: Python<'py>,
        file_ref: PyRef<'py, PyFileRef>,
        namespace: String,
        table: String,
        target_user_id: Option<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let state = self.state.clone();
        let file_ref = file_ref.inner.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let download = {
                let mut guard = state.lock().await;
                ensure_authenticated_locked(&mut guard).await?;
                guard
                    .client
                    .download_file(&file_ref, &namespace, &table, target_user_id.as_deref())
                    .await
                    .map_err(to_py_err)?
            };
            Python::attach(|py| {
                Py::new(py, PyFileDownload::from(download))
                    .map(|obj| obj.into_any())
                    .map_err(|e| PyRuntimeError::new_err(e.to_string()))
            })
        })
    }

    /// Insert a row into a table.
    ///
    /// Values are sent as parameterized query bindings ($1, $2, ...), so they
    /// are properly escaped by the server regardless of their contents.
    ///
    /// Args:
    ///     table: Fully qualified table name (e.g. "app.users")
    ///     data: Dict of column values
    ///
    /// Example:
    ///     await client.insert("app.messages", {
    ///         "role": "user",
    ///         "content": "hello"
    ///     })
    fn insert<'py>(
        &self,
        py: Python<'py>,
        table: String,
        data: Bound<'py, pyo3::types::PyDict>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let state = self.state.clone();

        // Build a parameterized INSERT: INSERT INTO t (c1, c2) VALUES ($1, $2)
        let mut columns: Vec<String> = Vec::new();
        let mut placeholders: Vec<String> = Vec::new();
        let mut params: Vec<QueryParam> = Vec::new();

        for (key, val) in data.iter() {
            let col: String = key.extract()?;
            columns.push(col);
            params.push(py_to_query_param(&val)?);
            placeholders.push(format!("${}", params.len()));
        }

        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            table,
            columns.join(", "),
            placeholders.join(", ")
        );

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            execute_with_auth_retry(&state, &sql, None, Some(params)).await?;
            Ok(Python::attach(|py| py.None()))
        })
    }

    /// Delete a row from a table by primary key value.
    ///
    /// Uses a parameterized query so the row id value is always safely escaped.
    ///
    /// Args:
    ///     table: Fully qualified table name (e.g. "app.users")
    ///     row_id: The primary key value of the row to delete
    ///     pk_column: Name of the primary key column (default: "id")
    ///
    /// Example:
    ///     await client.delete("app.messages", 12345)
    ///     await client.delete("app.sessions", "sess_abc", pk_column="session_id")
    #[pyo3(signature = (table, row_id, pk_column="id"))]
    fn delete<'py>(
        &self,
        py: Python<'py>,
        table: String,
        row_id: Bound<'py, PyAny>,
        pk_column: &str,
    ) -> PyResult<Bound<'py, PyAny>> {
        let state = self.state.clone();
        let id_value = py_to_query_param(&row_id)?;
        let pk_column = pk_column.to_string();

        let sql = format!("DELETE FROM {} WHERE {} = $1", table, pk_column);
        let params = vec![id_value];

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            execute_with_auth_retry(&state, &sql, None, Some(params)).await?;
            Ok(Python::attach(|py| py.None()))
        })
    }

    /// Disconnect the client.
    ///
    /// Example:
    ///     await client.disconnect()
    fn disconnect<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let state = self.state.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let guard = state.lock().await;
            guard.client.disconnect().await;
            Ok(Python::attach(|py| py.None()))
        })
    }

    /// Async context manager entry — enables `async with KalamClient(...) as client:`.
    fn __aenter__<'py>(slf: PyRef<'py, Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let slf_obj: Py<PyAny> = slf.into_pyobject(py)?.unbind().into();
        pyo3_async_runtimes::tokio::future_into_py(py, async move { Ok(slf_obj) })
    }

    /// Async context manager exit — automatically calls `disconnect()` on scope exit.
    #[pyo3(signature = (_exc_type, _exc_value, _traceback))]
    fn __aexit__<'py>(
        &self,
        py: Python<'py>,
        _exc_type: Bound<'py, PyAny>,
        _exc_value: Bound<'py, PyAny>,
        _traceback: Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let state = self.state.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let guard = state.lock().await;
            guard.client.disconnect().await;
            Ok(Python::attach(|py| py.None()))
        })
    }

    /// Create a topic consumer (Kafka-style pub/sub).
    ///
    /// Args:
    ///     topic: Topic name to consume from
    ///     group_id: Consumer group identifier (tracks per-group offsets)
    ///     start: Where to start reading: "earliest" or "latest" (default "latest")
    ///
    /// Example:
    ///     consumer = await client.consume("blog.posts", "summarizer-v1", start="earliest")
    ///     records = await consumer.poll()
    ///     for record in records:
    ///         print(record["payload"])
    ///     await consumer.commit()
    #[pyo3(signature = (topic, group_id, start="latest"))]
    fn consume<'py>(
        &self,
        py: Python<'py>,
        topic: String,
        group_id: String,
        start: &str,
    ) -> PyResult<Bound<'py, PyAny>> {
        let url = self.url.clone();
        let state = self.state.clone();

        let offset_reset = match start {
            "earliest" => AutoOffsetReset::Earliest,
            "latest" => AutoOffsetReset::Latest,
            other => {
                return Err(KalamConfigError::new_err(format!(
                    "start must be 'earliest' or 'latest', got '{other}'"
                )));
            },
        };

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            // Make sure we're authenticated so a JWT is available.
            let jwt = {
                let mut guard = state.lock().await;
                ensure_authenticated_locked(&mut guard).await?;
                guard.jwt.clone().ok_or_else(|| {
                    KalamAuthError::new_err("Cannot consume without authentication")
                })?
            };

            let consumer = TopicConsumer::builder()
                .base_url(&url)
                .jwt_token(jwt)
                .topic(&topic)
                .group_id(&group_id)
                .auto_offset_reset(offset_reset)
                .enable_auto_commit(false)
                .build()
                .map_err(to_py_err)?;

            Ok(Python::attach(|py| {
                Py::new(
                    py,
                    Consumer {
                        inner: Arc::new(Mutex::new(Some(consumer))),
                    },
                )
                .unwrap()
                .into_any()
            }))
        })
    }

    fn __repr__(&self) -> String {
        let status = match self.state.try_lock() {
            Ok(guard) if guard.authenticated => "connected",
            Ok(_) => "not connected",
            Err(_) => "busy",
        };
        format!("KalamClient(url='{}', {})", self.url, status)
    }

    /// Open a low-level live event stream for a SQL query.
    #[pyo3(signature = (sql, *, batch_size=None, last_rows=None, from_=None, auto_fetch_batches=None, on_checkpoint=None, on_error=None))]
    fn live_events<'py>(
        &self,
        py: Python<'py>,
        sql: String,
        batch_size: Option<usize>,
        last_rows: Option<u32>,
        from_: Option<Bound<'py, PyAny>>,
        auto_fetch_batches: Option<bool>,
        on_checkpoint: Option<Py<PyAny>>,
        on_error: Option<Py<PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let state = self.state.clone();
        let from = py_seq_id(from_)?;
        let config = live_subscription_config(
            sql,
            "events",
            batch_size,
            last_rows,
            from,
            auto_fetch_batches,
        );

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut guard = state.lock().await;
            ensure_authenticated_locked(&mut guard).await?;
            let manager = guard.client.live_events_with_config(config).await.map_err(to_py_err)?;

            Ok(Python::attach(|py| {
                Py::new(
                    py,
                    LiveEvents {
                        inner: Arc::new(Mutex::new(Some(manager))),
                        on_checkpoint,
                        on_error,
                    },
                )
                .unwrap()
                .into_any()
            }))
        })
    }

    /// Open a materialized live row stream for a SQL query.
    #[pyo3(signature = (sql, *, batch_size=None, last_rows=None, from_=None, auto_fetch_batches=None, limit=None, key_columns=None, on_checkpoint=None))]
    fn live<'py>(
        &self,
        py: Python<'py>,
        sql: String,
        batch_size: Option<usize>,
        last_rows: Option<u32>,
        from_: Option<Bound<'py, PyAny>>,
        auto_fetch_batches: Option<bool>,
        limit: Option<usize>,
        key_columns: Option<Vec<String>>,
        on_checkpoint: Option<Py<PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let state = self.state.clone();
        let from = py_seq_id(from_)?;
        let config =
            live_subscription_config(sql, "live", batch_size, last_rows, from, auto_fetch_batches);
        let live_config = LiveRowsConfig { limit, key_columns };

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut guard = state.lock().await;
            ensure_authenticated_locked(&mut guard).await?;
            let manager =
                guard.client.live_with_config(config, live_config).await.map_err(to_py_err)?;

            Ok(Python::attach(|py| {
                Py::new(
                    py,
                    LiveRows {
                        inner: Arc::new(Mutex::new(Some(manager))),
                        on_checkpoint,
                    },
                )
                .unwrap()
                .into_any()
            }))
        })
    }

    /// Open a materialized live row stream for `SELECT * FROM namespace.table`.
    #[pyo3(signature = (table, *, batch_size=None, last_rows=None, from_=None, auto_fetch_batches=None, limit=None, key_columns=None, on_checkpoint=None))]
    fn live_table<'py>(
        &self,
        py: Python<'py>,
        table: String,
        batch_size: Option<usize>,
        last_rows: Option<u32>,
        from_: Option<Bound<'py, PyAny>>,
        auto_fetch_batches: Option<bool>,
        limit: Option<usize>,
        key_columns: Option<Vec<String>>,
        on_checkpoint: Option<Py<PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let sql = table_live_sql(&table)?;
        self.live(
            py,
            sql,
            batch_size,
            last_rows,
            from_,
            auto_fetch_batches,
            limit,
            key_columns,
            on_checkpoint,
        )
    }
}

/// A topic consumer for Kafka-style pub/sub.
///
/// Use `async for record in consumer` to receive records, or call `await consumer.poll()`
/// for batch polling. Call `await consumer.commit()` to acknowledge processed records.
#[pyclass]
struct Consumer {
    inner: Arc<Mutex<Option<TopicConsumer>>>,
}

#[pymethods]
impl Consumer {
    /// Poll for the next batch of records.
    ///
    /// Returns a list of record dicts. Empty list means no records available.
    /// Records are NOT automatically marked as processed — you must call
    /// `mark_processed(record)` for each one you've handled before calling
    /// `commit()`. This prevents data loss if your handler fails mid-batch.
    fn poll<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut guard = inner.lock().await;
            let consumer =
                guard.as_mut().ok_or_else(|| KalamError::new_err("Consumer is closed"))?;

            let records = consumer.poll().await.map_err(to_py_err)?;

            Python::attach(|py| serialize_to_py(py, &records))
        })
    }

    /// Mark a record as successfully processed.
    ///
    /// After calling this for each record you've handled, call `commit()` to
    /// advance the server-side offset. Records that are polled but never
    /// marked will be redelivered on the next poll (after commit).
    fn mark_processed<'py>(
        &self,
        py: Python<'py>,
        record: Bound<'py, pyo3::types::PyDict>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();

        let offset: u64 = record
            .get_item("offset")?
            .ok_or_else(|| KalamError::new_err("record dict missing 'offset' field"))?
            .extract()?;

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut guard = inner.lock().await;
            let consumer =
                guard.as_mut().ok_or_else(|| KalamError::new_err("Consumer is closed"))?;

            // Reconstruct a minimal ConsumerRecord just to reuse the existing
            // offset-tracking API. Only `offset` is read inside `mark_processed`.
            let record = kalam_client::ConsumerRecord {
                topic_id: String::new(),
                topic_name: String::new(),
                partition_id: 0,
                offset,
                message_id: None,
                source_table: String::new(),
                op: kalam_client::TopicOp::Insert,
                timestamp_ms: 0,
                payload_mode: kalam_client::PayloadMode::Full,
                payload: Vec::new(),
            };
            consumer.mark_processed(&record);
            Ok(Python::attach(|py| py.None()))
        })
    }

    /// Commit the offsets of all records that have been marked as processed.
    fn commit<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut guard = inner.lock().await;
            let consumer =
                guard.as_mut().ok_or_else(|| KalamError::new_err("Consumer is closed"))?;

            consumer.commit_sync().await.map_err(to_py_err)?;
            Ok(Python::attach(|py| py.None()))
        })
    }

    /// Close the consumer.
    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut guard = inner.lock().await;
            *guard = None;
            Ok(Python::attach(|py| py.None()))
        })
    }

    /// Async context manager entry.
    fn __aenter__<'py>(slf: PyRef<'py, Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let slf_obj: Py<PyAny> = slf.into_pyobject(py)?.unbind().into();
        pyo3_async_runtimes::tokio::future_into_py(py, async move { Ok(slf_obj) })
    }

    /// Async context manager exit — auto-closes.
    #[pyo3(signature = (_exc_type, _exc_value, _traceback))]
    fn __aexit__<'py>(
        &self,
        py: Python<'py>,
        _exc_type: Bound<'py, PyAny>,
        _exc_value: Bound<'py, PyAny>,
        _traceback: Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut guard = inner.lock().await;
            *guard = None;
            Ok(Python::attach(|py| py.None()))
        })
    }

    fn __repr__(&self) -> String {
        // Non-blocking peek at the state; "unknown" if the mutex is busy
        let state = match self.inner.try_lock() {
            Ok(guard) if guard.is_some() => "open",
            Ok(_) => "closed",
            Err(_) => "busy",
        };
        format!("Consumer({state})")
    }
}

/// A low-level live event stream for a SQL query.
///
/// Use `async for event in live_events` to receive change events, or call
/// `await live_events.next()` manually.
#[pyclass]
struct LiveEvents {
    inner: Arc<Mutex<Option<SubscriptionManager>>>,
    on_checkpoint: Option<Py<PyAny>>,
    on_error: Option<Py<PyAny>>,
}

#[pymethods]
impl LiveEvents {
    /// Get the next change event.
    ///
    /// Raises StopAsyncIteration when the live event stream ends (connection
    /// closed or unsubscribed). This matches the `async for` iterator contract,
    /// so you can use either `await sub.next()` or `async for event in sub`
    /// and get consistent behaviour either way.
    fn next<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let on_checkpoint = self.on_checkpoint.as_ref().map(|callback| callback.clone_ref(py));
        let on_error = self.on_error.as_ref().map(|callback| callback.clone_ref(py));

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut guard = inner.lock().await;
            let manager = guard.as_mut().ok_or_else(|| {
                pyo3::exceptions::PyStopAsyncIteration::new_err("LiveEvents stream is closed")
            })?;

            match manager.next().await {
                Some(Ok(event)) => {
                    let checkpoint = change_event_checkpoint(&event);
                    let msg = event.to_server_message();
                    let json_msg = serde_json::to_value(&msg).map_err(|e| {
                        PyRuntimeError::new_err(format!("JSON serialization error: {e}"))
                    })?;
                    if let Some((subscription_id, seq_id)) = checkpoint {
                        call_checkpoint(on_checkpoint.as_ref(), subscription_id, Some(seq_id))?;
                    }
                    if matches!(
                        json_msg.get("type").and_then(serde_json::Value::as_str),
                        Some("error")
                    ) {
                        call_error(on_error.as_ref(), &json_msg)?;
                    }
                    Python::attach(|py| serde_json_value_to_py(py, &json_msg))
                },
                Some(Err(e)) => Err(to_py_err(e)),
                None => {
                    Err(pyo3::exceptions::PyStopAsyncIteration::new_err("LiveEvents stream ended"))
                },
            }
        })
    }

    /// Close the live event stream.
    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut guard = inner.lock().await;
            if let Some(mut manager) = guard.take() {
                let _ = manager.close().await;
            }
            Ok(Python::attach(|py| py.None()))
        })
    }

    /// Async context manager entry — enables `async with await client.live_events(...) as events:`.
    fn __aenter__<'py>(slf: PyRef<'py, Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let slf_obj: Py<PyAny> = slf.into_pyobject(py)?.unbind().into();
        pyo3_async_runtimes::tokio::future_into_py(py, async move { Ok(slf_obj) })
    }

    /// Async context manager exit — automatically closes the live event stream.
    #[pyo3(signature = (_exc_type, _exc_value, _traceback))]
    fn __aexit__<'py>(
        &self,
        py: Python<'py>,
        _exc_type: Bound<'py, PyAny>,
        _exc_value: Bound<'py, PyAny>,
        _traceback: Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut guard = inner.lock().await;
            if let Some(mut manager) = guard.take() {
                let _ = manager.close().await;
            }
            Ok(Python::attach(|py| py.None()))
        })
    }

    fn __repr__(&self) -> String {
        let state = match self.inner.try_lock() {
            Ok(guard) if guard.is_some() => "open",
            Ok(_) => "closed",
            Err(_) => "busy",
        };
        format!("LiveEvents({state})")
    }

    /// Async iterator support: `async for event in live_events`.
    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __anext__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let on_checkpoint = self.on_checkpoint.as_ref().map(|callback| callback.clone_ref(py));
        let on_error = self.on_error.as_ref().map(|callback| callback.clone_ref(py));

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut guard = inner.lock().await;
            let manager = guard
                .as_mut()
                .ok_or_else(|| pyo3::exceptions::PyStopAsyncIteration::new_err(""))?;

            match manager.next().await {
                Some(Ok(event)) => {
                    let checkpoint = change_event_checkpoint(&event);
                    let msg = event.to_server_message();
                    let json_msg = serde_json::to_value(&msg).map_err(|e| {
                        PyRuntimeError::new_err(format!("JSON serialization error: {e}"))
                    })?;
                    if let Some((subscription_id, seq_id)) = checkpoint {
                        call_checkpoint(on_checkpoint.as_ref(), subscription_id, Some(seq_id))?;
                    }
                    if matches!(
                        json_msg.get("type").and_then(serde_json::Value::as_str),
                        Some("error")
                    ) {
                        call_error(on_error.as_ref(), &json_msg)?;
                    }
                    Python::attach(|py| serde_json_value_to_py(py, &json_msg))
                },
                Some(Err(e)) => Err(to_py_err(e)),
                None => Err(pyo3::exceptions::PyStopAsyncIteration::new_err("")),
            }
        })
    }
}

/// A materialized live row stream for a SQL query or table.
#[pyclass]
struct LiveRows {
    inner: Arc<Mutex<Option<LiveRowsSubscription>>>,
    on_checkpoint: Option<Py<PyAny>>,
}

#[pymethods]
impl LiveRows {
    /// Get the next materialized row snapshot.
    fn next<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let on_checkpoint = self.on_checkpoint.as_ref().map(|callback| callback.clone_ref(py));

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut guard = inner.lock().await;
            let manager = guard.as_mut().ok_or_else(|| {
                pyo3::exceptions::PyStopAsyncIteration::new_err("LiveRows stream is closed")
            })?;

            match manager.next().await {
                Some(Ok(LiveRowsEvent::Rows {
                    subscription_id,
                    rows,
                    last_seq_id,
                })) => {
                    call_checkpoint(on_checkpoint.as_ref(), &subscription_id, last_seq_id)?;
                    Python::attach(|py| serialize_to_py(py, &rows))
                },
                Some(Ok(LiveRowsEvent::Error { code, message, .. })) => {
                    Err(KalamServerError::new_err(format!("{code}: {message}")))
                },
                Some(Err(e)) => Err(to_py_err(e)),
                None => {
                    Err(pyo3::exceptions::PyStopAsyncIteration::new_err("LiveRows stream ended"))
                },
            }
        })
    }

    /// Close the live row stream.
    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut guard = inner.lock().await;
            if let Some(mut manager) = guard.take() {
                let _ = manager.close().await;
            }
            Ok(Python::attach(|py| py.None()))
        })
    }

    fn __aenter__<'py>(slf: PyRef<'py, Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let slf_obj: Py<PyAny> = slf.into_pyobject(py)?.unbind().into();
        pyo3_async_runtimes::tokio::future_into_py(py, async move { Ok(slf_obj) })
    }

    #[pyo3(signature = (_exc_type, _exc_value, _traceback))]
    fn __aexit__<'py>(
        &self,
        py: Python<'py>,
        _exc_type: Bound<'py, PyAny>,
        _exc_value: Bound<'py, PyAny>,
        _traceback: Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.close(py)
    }

    fn __repr__(&self) -> String {
        let state = match self.inner.try_lock() {
            Ok(guard) if guard.is_some() => "open",
            Ok(_) => "closed",
            Err(_) => "busy",
        };
        format!("LiveRows({state})")
    }

    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __anext__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.next(py)
    }
}

/// Convert any serde-serializable value directly to a Python object.
///
/// Goes via serde_json::Value as an intermediate, but avoids the extra
/// string-serialize + json.loads round-trip that shows up in profiles for
/// large query responses.
fn serialize_to_py<T: serde::Serialize>(py: Python<'_>, value: &T) -> PyResult<Py<PyAny>> {
    let json_value = serde_json::to_value(value)
        .map_err(|e| PyRuntimeError::new_err(format!("JSON serialization error: {e}")))?;
    serde_json_value_to_py(py, &json_value)
}

/// Convert a Python list of values into typed query parameters.
fn py_params_to_query_params(
    params: Option<Bound<'_, pyo3::types::PyList>>,
) -> PyResult<Option<Vec<QueryParam>>> {
    let Some(list) = params else {
        return Ok(None);
    };

    let mut out = Vec::with_capacity(list.len());
    for item in list.iter() {
        out.push(py_to_query_param(&item)?);
    }
    Ok(Some(out))
}

/// Convert a single Python value into a [`QueryParam`].
fn py_to_query_param(value: &Bound<'_, PyAny>) -> PyResult<QueryParam> {
    if let Ok(file_ref) = value.extract::<PyRef<PyFileRef>>() {
        return Ok(QueryParam::File(file_ref.inner.clone()));
    }
    Ok(from_json_value(py_to_json_value(value)?))
}

/// Convert a single Python value into a serde_json::Value.
fn py_to_json_value(value: &Bound<'_, PyAny>) -> PyResult<serde_json::Value> {
    if value.is_none() {
        return Ok(serde_json::Value::Null);
    }
    if let Ok(b) = value.extract::<bool>() {
        return Ok(serde_json::Value::Bool(b));
    }
    if let Ok(i) = value.extract::<i64>() {
        return Ok(serde_json::Value::Number(i.into()));
    }
    if let Ok(f) = value.extract::<f64>() {
        return serde_json::Number::from_f64(f)
            .map(serde_json::Value::Number)
            .ok_or_else(|| PyRuntimeError::new_err(format!("Cannot serialize float: {f}")));
    }
    if let Ok(s) = value.extract::<String>() {
        return Ok(serde_json::Value::String(s));
    }
    if let Ok(list) = value.cast::<pyo3::types::PyList>() {
        let mut arr = Vec::with_capacity(list.len());
        for item in list.iter() {
            arr.push(py_to_json_value(&item)?);
        }
        return Ok(serde_json::Value::Array(arr));
    }
    if let Ok(dict) = value.cast::<pyo3::types::PyDict>() {
        let mut map = serde_json::Map::new();
        for (k, v) in dict.iter() {
            let key: String = k.extract()?;
            map.insert(key, py_to_json_value(&v)?);
        }
        return Ok(serde_json::Value::Object(map));
    }

    Err(PyRuntimeError::new_err(format!(
        "Unsupported parameter type: {}",
        value.get_type().name()?
    )))
}

/// Convert a serde_json::Value to a Python object.
fn serde_json_value_to_py(py: Python<'_>, value: &serde_json::Value) -> PyResult<Py<PyAny>> {
    if let Some(file_ref) = FileRef::from_json_value(value) {
        return Ok(Py::new(py, PyFileRef::from(file_ref))?.into());
    }

    match value {
        serde_json::Value::Null => Ok(py.None()),
        serde_json::Value::Bool(b) => Ok((*b).into_pyobject(py)?.to_owned().unbind().into()),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(i.into_pyobject(py)?.unbind().into())
            } else if let Some(f) = n.as_f64() {
                Ok(f.into_pyobject(py)?.unbind().into())
            } else {
                Ok(py.None())
            }
        },
        serde_json::Value::String(s) => Ok(s.into_pyobject(py)?.unbind().into()),
        serde_json::Value::Array(arr) => {
            let list = pyo3::types::PyList::empty(py);
            for item in arr {
                list.append(serde_json_value_to_py(py, item)?)?;
            }
            Ok(list.unbind().into())
        },
        serde_json::Value::Object(map) => {
            let dict = pyo3::types::PyDict::new(py);
            for (k, v) in map {
                dict.set_item(k, serde_json_value_to_py(py, v)?)?;
            }
            Ok(dict.unbind().into())
        },
    }
}

/// KalamDB Python SDK — internal Rust module (exposed as `kalamdb._native`).
#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();

    m.add_class::<KalamClient>()?;
    m.add_class::<Auth>()?;
    m.add_class::<PyFileRef>()?;
    m.add("FileRef", py.get_type::<PyFileRef>())?;
    m.add_class::<PyFileUpload>()?;
    m.add("FileUpload", py.get_type::<PyFileUpload>())?;
    m.add_class::<PyFileDownload>()?;
    m.add("FileDownload", py.get_type::<PyFileDownload>())?;
    m.add_class::<LiveEvents>()?;
    m.add_class::<LiveRows>()?;
    m.add_class::<Consumer>()?;

    m.add("KalamError", py.get_type::<KalamError>())?;
    m.add("KalamConnectionError", py.get_type::<KalamConnectionError>())?;
    m.add("KalamAuthError", py.get_type::<KalamAuthError>())?;
    m.add("KalamServerError", py.get_type::<KalamServerError>())?;
    m.add("KalamConfigError", py.get_type::<KalamConfigError>())?;

    Ok(())
}
