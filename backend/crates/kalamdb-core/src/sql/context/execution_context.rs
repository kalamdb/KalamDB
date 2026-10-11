use std::sync::Arc;

use datafusion::prelude::SessionContext;
use kalamdb_commons::{models::ReadContext, NamespaceId, Role, TransactionId, UserId};
use kalamdb_session::AuthSession;
use kalamdb_session_datafusion::{install_extension, SessionUserContext};
use once_cell::sync::OnceCell;

/// Unified execution context for SQL queries
///
/// Combines authenticated session information with DataFusion session management
/// and query-specific state (namespace, caching).
#[derive(Clone)]
pub struct ExecutionContext {
    /// Authenticated session with user identity and metadata
    auth_session:            AuthSession,
    /// Optional namespace for this query execution
    namespace_id:            Option<NamespaceId>,
    /// Optional existing transaction supplied by a connection-scoped caller.
    transaction_id:          Option<TransactionId>,
    /// When true, STREAM DML from this context autocommits instead of joining
    /// the wrapping request transaction (procedure CALL / topic trigger).
    allow_stream_autocommit: bool,
    /// Base SessionContext from AppContext (tables already registered)
    /// We extract SessionState from this and inject user_id to create per-request SessionContext
    base_session_context:    Arc<SessionContext>,
    /// Cached per-request SessionState with user context injected
    ///
    /// This avoids repeated allocations when a single request needs multiple
    /// SessionContext instances (retries, planning fallbacks) while keeping
    /// user isolation intact.
    session_context_cache:   Arc<OnceCell<SessionContext>>,
}

impl ExecutionContext {
    /// Create a new ExecutionContext with base SessionContext
    ///
    /// # Arguments
    /// * `user_id` - User ID executing the query
    /// * `user_role` - User's role for authorization
    /// * `base_session_context` - Base SessionContext from AppContext (tables already registered)
    ///
    /// # Note
    /// The base_session_context contains all registered table providers.
    /// When executing queries, call `create_session_with_user()` to get a
    /// SessionContext with user_id injected for per-user filtering.
    pub fn new(
        user_id: UserId,
        user_role: Role,
        base_session_context: Arc<SessionContext>,
    ) -> Self {
        Self::bare(AuthSession::new(user_id, user_role), None, base_session_context)
    }

    /// Create ExecutionContext from an existing AuthSession
    pub fn from_session(
        auth_session: AuthSession,
        base_session_context: Arc<SessionContext>,
    ) -> Self {
        Self::bare(auth_session, None, base_session_context)
    }

    pub fn with_namespace(
        user_id: UserId,
        user_role: Role,
        namespace_id: NamespaceId,
        base_session_context: Arc<SessionContext>,
    ) -> Self {
        Self::bare(AuthSession::new(user_id, user_role), Some(namespace_id), base_session_context)
    }

    fn bare(
        auth_session: AuthSession,
        namespace_id: Option<NamespaceId>,
        base_session_context: Arc<SessionContext>,
    ) -> Self {
        Self {
            auth_session,
            namespace_id,
            transaction_id: None,
            allow_stream_autocommit: false,
            base_session_context,
            session_context_cache: Arc::new(OnceCell::new()),
        }
    }

    #[inline]
    pub fn is_admin(&self) -> bool {
        self.auth_session.is_admin()
    }
    #[inline]
    pub fn is_system(&self) -> bool {
        self.auth_session.is_system()
    }

    /// Check if this is an anonymous user (not authenticated)
    ///
    /// Anonymous users have limited permissions:
    /// - Can only SELECT from public tables
    /// - Cannot CREATE, ALTER, DROP, INSERT, UPDATE, or DELETE
    #[inline]
    pub fn is_anonymous(&self) -> bool {
        self.auth_session.is_anonymous()
    }

    #[inline]
    pub fn user_id(&self) -> &UserId {
        self.auth_session.user_id()
    }
    #[inline]
    pub fn user_role(&self) -> Role {
        self.auth_session.role()
    }
    #[inline]
    pub(crate) fn read_context(&self) -> ReadContext {
        self.auth_session.read_context()
    }
    #[inline]
    pub fn request_id(&self) -> Option<&str> {
        self.auth_session.request_id()
    }
    #[inline]
    pub fn ip_address(&self) -> Option<&str> {
        self.auth_session.ip_address()
    }
    // Builder methods for Phase 3
    pub fn with_request_id(mut self, request_id: impl Into<Arc<str>>) -> Self {
        self.auth_session = self.auth_session.with_request_id(request_id);
        self
    }

    /// Set the namespace for this execution context
    pub fn with_namespace_id(mut self, namespace_id: NamespaceId) -> Self {
        self.namespace_id = Some(namespace_id);
        self.session_context_cache = Arc::new(OnceCell::new());
        self
    }

    pub fn with_transaction_id(mut self, transaction_id: TransactionId) -> Self {
        self.transaction_id = Some(transaction_id);
        self.session_context_cache = Arc::new(OnceCell::new());
        self
    }

    pub fn without_transaction_id(mut self) -> Self {
        self.transaction_id = None;
        self.session_context_cache = Arc::new(OnceCell::new());
        self
    }

    pub fn with_stream_autocommit(mut self) -> Self {
        self.allow_stream_autocommit = true;
        self
    }

    #[inline]
    pub fn transaction_id(&self) -> Option<&TransactionId> {
        self.transaction_id.as_ref()
    }

    #[inline]
    pub fn allows_stream_autocommit(&self) -> bool {
        self.allow_stream_autocommit
    }

    /// Clone this context with an explicit effective identity while preserving
    /// namespace and request metadata.
    ///
    /// Cross-user callers must be authorized by SqlImpersonationService before
    /// reaching this method.
    pub fn with_effective_identity(&self, user_id: UserId, role: Role) -> Self {
        let mut auth_session = self.auth_session.clone();
        auth_session.user_context.user_id = user_id;
        auth_session.user_context.role = role;

        let mut ctx = Self::bare(
            auth_session,
            self.namespace_id.clone(),
            Arc::clone(&self.base_session_context),
        );
        ctx.transaction_id = self.transaction_id.clone();
        ctx.allow_stream_autocommit = self.allow_stream_autocommit;
        ctx
    }

    /// Set the read context (client vs internal)
    ///
    /// Use `ReadContext::Internal` for WebSocket subscriptions on followers
    /// to bypass leader-only read checks while still applying RLS.
    pub fn with_read_context(mut self, read_context: ReadContext) -> Self {
        self.auth_session = self.auth_session.with_read_context_mode(read_context);
        self.session_context_cache = Arc::new(OnceCell::new());
        self
    }

    pub(crate) fn build_user_session_state(&self) -> datafusion::execution::context::SessionState {
        // SessionContext::state() already clones SessionState once; avoid cloning it again.
        // Keep per-user options/extensions isolated while shared internals stay Arc-backed.
        // (shared internals like RuntimeEnv remain Arc-backed)
        crate::sql::datafusion_session::DataFusionSessionFactory::ensure_extended_functions(
            &self.base_session_context,
        );
        let mut session_state = self.base_session_context.state();

        // Inject current user_id, role, and read_context into session config extensions
        // TableProviders will read this during scan() for per-user filtering and leader check
        // Use the read_context from this ExecutionContext (defaults to Client)
        let session_user_context = SessionUserContext::new(
            self.auth_session.user_id().clone(),
            self.auth_session.role(),
            self.auth_session.read_context(),
        );

        install_extension(&mut session_state, session_user_context);

        // Override default_schema if namespace_id is set on this context
        if let Some(ref ns) = self.namespace_id {
            session_state.config_mut().options_mut().catalog.default_schema =
                ns.as_str().to_string();
        }

        session_state
    }

    /// Per-request session with the current user injected.
    ///
    /// The first call builds one `SessionState` and caches it for this request.
    /// Later calls clone that context. Shared catalogs and the runtime stay
    /// behind `Arc`; `config_mut` copies config only when a caller changes it.
    /// A namespace on this context overrides `default_schema`.
    pub fn create_session_with_user(&self) -> SessionContext {
        let session = self
            .session_context_cache
            .get_or_init(|| SessionContext::new_with_state(self.build_user_session_state()));
        session.clone()
    }

    /// Get the current default namespace (schema) from DataFusion session config
    ///
    /// This reads `datafusion.catalog.default_schema` from the request-scoped
    /// session configuration.
    /// The default schema is set to "default" initially and can be changed within
    /// the current request or multi-statement batch using:
    /// - `USE namespace`
    /// - `USE NAMESPACE namespace`
    /// - `SET NAMESPACE namespace`
    ///
    /// # Returns
    /// The current default namespace as a NamespaceId (defaults to "default")
    pub fn default_namespace(&self) -> NamespaceId {
        if let Some(session) = self.session_context_cache.get() {
            let default_schema =
                session.state_ref().read().config().options().catalog.default_schema.clone();
            return NamespaceId::from_session_schema(&default_schema, self.namespace_id.as_ref());
        }

        self.namespace_id.clone().unwrap_or_default()
    }
}
