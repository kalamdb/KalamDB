//! Shared workflow authentication resolution for SQL, schema gen, and dev precheck.

mod credentials;
mod provider;
mod verify;

pub(crate) use credentials::{resolve_local_dev_root_password, workflow_auth_config_error};
pub(crate) use provider::resolve_workflow_auth_provider;
#[cfg(test)]
pub(crate) use verify::local_dev_credentials_from_login;
pub(crate) use verify::{
    login_with_credentials, save_local_dev_credentials, verify_jwt_auth, verify_workflow_auth,
};
