use std::{future::Future, pin::Pin, sync::Mutex};

use flutter_rust_bridge::DartFnFuture;
use kalam_client::{AuthProvider, DynamicAuthProvider, KalamLinkError, Result};

use crate::models::DartAuthProvider;

pub(crate) struct WebSocketAuth {
    pending:  Mutex<Option<AuthProvider>>,
    provider: Box<dyn Fn() -> DartFnFuture<Option<DartAuthProvider>> + Send + Sync>,
}

impl WebSocketAuth {
    pub(crate) fn new(
        initial: AuthProvider,
        provider: impl Fn() -> DartFnFuture<Option<DartAuthProvider>> + Send + Sync + 'static,
    ) -> Self {
        Self {
            pending:  Mutex::new(Some(initial)),
            provider: Box::new(provider),
        }
    }

    pub(crate) fn update(&self, auth: AuthProvider) {
        *self.pending.lock().unwrap() = Some(auth);
    }
}

impl DynamicAuthProvider for WebSocketAuth {
    fn get_auth(&self) -> Pin<Box<dyn Future<Output = Result<AuthProvider>> + Send + '_>> {
        let pending = self.pending.lock().unwrap().take();
        Box::pin(async move {
            if let Some(auth) = pending {
                return Ok(auth);
            }
            (self.provider)().await.map(DartAuthProvider::into_native).ok_or_else(|| {
                KalamLinkError::AuthenticationError("Dart credential provider failed".into())
            })
        })
    }
}
