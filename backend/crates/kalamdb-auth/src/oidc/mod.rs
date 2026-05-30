//! Internal OIDC support backed by `openidconnect` provider metadata and verifiers.

mod client;
mod device;
mod error;
mod http;

pub(crate) use client::{
    get_or_discover_oidc_client, oidc_client_cache, OidcClientCache, OidcClientHandle,
};
pub(crate) use device::{DeviceBrokerSession, DeviceBrokerState, DeviceBrokerStatus};
pub(crate) use error::OidcError;
pub(crate) use http::default_oidc_http_client;
