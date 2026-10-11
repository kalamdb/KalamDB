use std::any::Any;

use datafusion::{
    common::config::{ConfigEntry, ConfigExtension, ExtensionOptions},
    execution::context::SessionState,
};
use kalamdb_commons::{
    models::{ReadContext, Role, UserId},
    PolicyCommand,
};
use kalamdb_session::UserContext;

/// Install a config extension on a session state.
///
/// Callers share this so user, transaction, and diagnostics extensions are
/// attached the same way.
#[inline]
pub fn install_extension<T: ConfigExtension>(state: &mut SessionState, extension: T) {
    state.config_mut().options_mut().extensions.insert(extension);
}

macro_rules! empty_config_extension {
    ($ty:ty, $prefix:literal) => {
        impl ExtensionOptions for $ty {
            fn as_any(&self) -> &dyn Any {
                self
            }

            fn as_any_mut(&mut self) -> &mut dyn Any {
                self
            }

            fn cloned(&self) -> Box<dyn ExtensionOptions> {
                Box::new(self.clone())
            }

            fn set(&mut self, _key: &str, _value: &str) -> datafusion::common::Result<()> {
                Ok(())
            }

            fn entries(&self) -> Vec<ConfigEntry> {
                Vec::new()
            }
        }

        impl ConfigExtension for $ty {
            const PREFIX: &'static str = $prefix;
        }
    };
}

/// Session-level user context stored in DataFusion config extensions.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionUserContext {
    pub user_id:      UserId,
    pub role:         Role,
    pub read_context: ReadContext,
}

impl Default for SessionUserContext {
    fn default() -> Self {
        Self::from(UserContext::default())
    }
}

impl SessionUserContext {
    #[inline]
    pub fn new(user_id: UserId, role: Role, read_context: ReadContext) -> Self {
        Self {
            user_id,
            role,
            read_context,
        }
    }

    #[inline]
    pub fn into_user_context(self) -> UserContext {
        UserContext::new(self.user_id, self.role, self.read_context)
    }
}

impl From<UserContext> for SessionUserContext {
    fn from(value: UserContext) -> Self {
        Self {
            user_id:      value.user_id,
            role:         value.role,
            read_context: value.read_context,
        }
    }
}

impl From<&UserContext> for SessionUserContext {
    fn from(value: &UserContext) -> Self {
        Self {
            user_id:      value.user_id.clone(),
            role:         value.role,
            read_context: value.read_context,
        }
    }
}

empty_config_extension!(SessionUserContext, "kalamdb_user");

/// Per-operation RLS command used while a provider performs DML visibility scans.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RlsCommandContext {
    pub command: PolicyCommand,
}

impl Default for RlsCommandContext {
    fn default() -> Self {
        Self {
            command: PolicyCommand::Select,
        }
    }
}

empty_config_extension!(RlsCommandContext, "kalamdb_rls_command");

/// Session flag used to opt into scan-level diagnostics for `EXPLAIN ANALYZE`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct ScanDiagnosticsContext {
    enabled: bool,
}

impl ScanDiagnosticsContext {
    #[inline]
    pub fn enabled() -> Self {
        Self { enabled: true }
    }

    #[inline]
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

empty_config_extension!(ScanDiagnosticsContext, "kalamdb_scan_diagnostics");
