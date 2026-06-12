//! Lightweight HTTP runtime state shared by Actix workers.

use std::io::IsTerminal;
use std::sync::Arc;

use actix_web::web;
use anyhow::Result;
use kalamdb_api::{limiter::RateLimiter, ui::UiRuntimeConfig};
use kalamdb_auth::UserRepository;
use kalamdb_configs::{AuthSettings, CorsSettings, ServerConfig};
use kalamdb_core::{
    app_context::AppContext,
    sql::{datafusion_session::DataFusionSessionFactory, executor::SqlExecutor},
};
use kalamdb_live::{ConnectionsManager, LiveQueryManager};

use crate::{
    lifecycle::ApplicationComponents, middleware::ConnectionProtection,
    startup::configure_auth_runtime,
};

#[derive(Clone, Copy)]
pub enum AuthRuntimeMode {
    Configure,
    AlreadyConfigured,
}

#[derive(Clone)]
pub struct HttpRuntimeState {
    pub app_context: web::Data<Arc<AppContext>>,
    pub session_factory: web::Data<Arc<DataFusionSessionFactory>>,
    pub sql_executor: web::Data<Arc<SqlExecutor>>,
    pub rate_limiter: web::Data<Arc<RateLimiter>>,
    pub live_query_manager: web::Data<Arc<LiveQueryManager>>,
    pub user_repo: web::Data<Arc<dyn UserRepository>>,
    pub connection_registry: web::Data<Arc<ConnectionsManager>>,
    pub auth_settings: web::Data<AuthSettings>,
    pub connection_protection: ConnectionProtection,
    pub cors_settings: Arc<CorsSettings>,
    pub ui_path: Option<String>,
    pub ui_runtime_config: UiRuntimeConfig,
    ui_status: &'static str,
}

impl HttpRuntimeState {
    pub fn new(
        config: &ServerConfig,
        components: &ApplicationComponents,
        app_context: Arc<AppContext>,
        auth_runtime_mode: AuthRuntimeMode,
    ) -> Result<Self> {
        if matches!(auth_runtime_mode, AuthRuntimeMode::Configure) {
            configure_auth_runtime(config)?;
        }

        let ui_path = config.server.ui_path.clone();
        let ui_status = if kalamdb_api::routes::is_embedded_ui_available() {
            "embedded in binary"
        } else if ui_path.is_some() {
            "filesystem"
        } else {
            "disabled"
        };

        Ok(Self {
            app_context: web::Data::new(app_context),
            session_factory: web::Data::new(components.session_factory.clone()),
            sql_executor: web::Data::new(components.sql_executor.clone()),
            rate_limiter: web::Data::new(components.rate_limiter.clone()),
            live_query_manager: web::Data::new(components.live_query_manager.clone()),
            user_repo: web::Data::new(components.user_repo.clone()),
            connection_registry: web::Data::new(components.connection_registry.clone()),
            auth_settings: web::Data::new(config.auth.clone()),
            connection_protection: ConnectionProtection::from_server_config(config),
            cors_settings: Arc::new(config.security.cors.clone()),
            ui_path,
            ui_runtime_config: UiRuntimeConfig::new(config.server.configured_public_origin()),
            ui_status,
        })
    }

    pub fn ui_status(&self) -> &'static str {
        self.ui_status
    }
}

fn terminal_hyperlinks_enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal()
}

pub(crate) fn should_print_terminal_hyperlinks(config: &ServerConfig) -> bool {
    config.logging.log_to_console && terminal_hyperlinks_enabled()
}

/// Format `url` as an OSC-8 terminal hyperlink.
fn format_terminal_hyperlink(url: &str) -> String {
    format!("\x1b]8;;{url}\x1b\\{url}\x1b]8;;\x1b\\")
}

/// Plain Admin UI segment for log files and non-interactive output.
pub fn format_startup_ui_status_plain(config: &ServerConfig, ui_status: &str) -> String {
    if ui_status == "disabled" {
        return "disabled".to_string();
    }

    let public_url = config.server.admin_ui_url();
    let mut message = format!("{ui_status} at {public_url}");

    if config.server.configured_public_origin().is_some() {
        let local_url = config.server.local_admin_ui_url();
        if local_url != public_url {
            message.push_str(&format!(" | local: {local_url}"));
        }
    }

    message
}

/// Admin UI segment with OSC-8 hyperlinks for direct terminal output.
pub fn format_startup_ui_status_with_links(config: &ServerConfig, ui_status: &str) -> String {
    if ui_status == "disabled" {
        return "disabled".to_string();
    }

    let public_url = config.server.admin_ui_url();
    let public_link = format_terminal_hyperlink(&public_url);
    let mut message = format!("{ui_status} at {public_link}");

    if config.server.configured_public_origin().is_some() {
        let local_url = config.server.local_admin_ui_url();
        if local_url != public_url {
            let local_link = format_terminal_hyperlink(&local_url);
            message.push_str(&format!(" | local: {local_link}"));
        }
    }

    message
}

#[cfg(test)]
mod tests {
    use kalamdb_configs::ServerConfig;

    use super::*;

    #[test]
    fn startup_ui_status_disabled_when_ui_unavailable() {
        let config = ServerConfig::default();

        assert_eq!(format_startup_ui_status_plain(&config, "disabled"), "disabled");
    }

    #[test]
    fn startup_ui_status_uses_public_origin_when_configured() {
        let mut config = ServerConfig::default();
        config.server.port = 2900;
        config.server.host = "127.0.0.1".to_string();
        config.server.public_origin = Some("https://db.example.com".to_string());

        let status = format_startup_ui_status_plain(&config, "embedded in binary");

        assert!(status.contains("embedded in binary at "));
        assert!(status.contains("https://db.example.com/ui"));
        assert!(status.contains("local: http://127.0.0.1:2900/ui"));
    }

    #[test]
    fn startup_ui_status_uses_localhost_fallback_without_public_origin() {
        let mut config = ServerConfig::default();
        config.server.port = 2900;
        config.server.host = "127.0.0.1".to_string();

        let status = format_startup_ui_status_plain(&config, "embedded in binary");

        assert_eq!(status, "embedded in binary at http://localhost:2900/ui");
    }

    #[test]
    fn startup_ui_status_with_links_includes_osc8_sequences() {
        let mut config = ServerConfig::default();
        config.server.port = 2900;

        let status = format_startup_ui_status_with_links(&config, "embedded in binary");

        assert!(status.contains("\x1b]8;;http://localhost:2900/ui\x1b\\"));
    }

    #[test]
    fn terminal_hyperlink_plain_when_not_tty() {
        let url = "http://localhost:2900/ui";
        let link = format_terminal_hyperlink(url);

        assert!(link.contains("\x1b]8;;http://localhost:2900/ui\x1b\\"));
        assert!(link.contains("http://localhost:2900/ui"));
    }
}
