//! Authentication request and response models
//!
//! This module contains type-safe models for all authentication endpoints.

mod error_response;
mod login_options;
mod login_response;
mod oidc_device;
mod oidc_exchange;
mod setup_request;
mod setup_response;
mod user_info;

pub use error_response::AuthErrorResponse;
pub use kalamdb_auth::LoginRequest;
pub use login_options::{
    AuthLoginOptionsResponse, LocalLoginOptions, OidcDeviceFlowOptions, OidcLoginOptions,
};
pub use login_response::LoginResponse;
pub use oidc_device::{
    OidcDevicePollRequest, OidcDevicePollResponse, OidcDevicePollStatus, OidcDeviceStartRequest,
    OidcDeviceStartResponse,
};
pub use oidc_exchange::{OidcCodeExchangeRequest, OidcTokenExchangeRequest};
pub use setup_request::ServerSetupRequest;
pub use setup_response::ServerSetupResponse;
pub use user_info::{CurrentUserResponse, UserInfo};
