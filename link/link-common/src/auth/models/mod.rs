//! Authentication and identity models.

pub mod login;
pub mod setup_models;

pub use kalamdb_commons::WsAuthCredentials;
pub use login::{LoginRequest, LoginResponse, LoginUserInfo};
pub use setup_models::{
    ServerSetupRequest, ServerSetupResponse, SetupStatusResponse, SetupUserInfo,
};
