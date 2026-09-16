//! Internal OIDC support backed by `openidconnect` provider metadata and verifiers.

mod client;
mod device;
mod error;
mod http;

pub(crate) use client::{
    get_or_discover_oidc_client, invalidate_oidc_client, oidc_client_cache, oidc_jwks_may_be_stale,
    OidcClientCache, OidcClientHandle,
};
pub(crate) use device::{DeviceBrokerSession, DeviceBrokerState, DeviceBrokerStatus};
pub(crate) use error::OidcError;
pub(crate) use http::default_oidc_http_client;
