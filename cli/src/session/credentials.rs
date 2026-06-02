use colored::Colorize;
use kalam_client::credentials::{CredentialStore, Credentials};

use super::CLISession;
use crate::error::{CLIError, Result};

impl CLISession {
    pub(in crate::session) fn show_credentials(&self) {
        match (&self.instance, &self.credential_store) {
            (Some(instance), Some(store)) => {
                match store
                    .lock()
                    .expect("credential store lock should not be poisoned")
                    .get_credentials(instance)
                {
                    Ok(Some(creds)) => {
                        println!("{}", "Stored Credentials".bold().cyan());
                        println!("  Instance: {}", creds.instance.green());
                        if let Some(ref name) = creds.name {
                            println!("  Name: {}", name.green());
                        }
                        if let Some(ref user) = creds.user {
                            println!("  User: {}", user.as_str().green());
                        }
                        if let Some(ref email) = creds.email {
                            println!("  Email: {}", email.green());
                        }
                        println!("  JWT Token: {}", "[redacted]".dimmed());
                        if let Some(ref expires) = creds.expires_at {
                            let expired_marker = if creds.is_expired() {
                                " (EXPIRED)".red().to_string()
                            } else {
                                String::new()
                            };
                            println!("  Expires: {}{}", expires.green(), expired_marker);
                        }
                        if let Some(ref server_url) = creds.server_url {
                            println!("  Server URL: {}", server_url.green());
                        }
                        println!();
                        println!("{}", "Security Note:".yellow().bold());
                        println!(
                            "  Credentials are stored in: {}",
                            crate::credentials::FileCredentialStore::default_path()
                                .display()
                                .to_string()
                                .dimmed()
                        );
                        #[cfg(unix)]
                        println!("{}", "  File permissions: 0600 (owner read/write only)".dimmed());
                    },
                    Ok(None) => {
                        println!("{}", "No credentials stored for this instance".yellow());
                        println!("Use --user and --password to login and store credentials");
                    },
                    Err(e) => {
                        eprintln!("{} {}", "Error loading credentials:".red(), e);
                    },
                }
            },
            (None, _) => {
                println!("{}", "Credential management not available".yellow());
                println!("Instance name not set for this session");
            },
            (_, None) => {
                println!("{}", "Credential store not available".yellow());
                println!("Credential storage was not initialized for this session");
            },
        }
    }

    pub(in crate::session) async fn update_credentials(
        &mut self,
        user: String,
        password: String,
    ) -> Result<()> {
        match (&self.instance, &mut self.credential_store) {
            (Some(instance), Some(store)) => {
                println!("{}", "Logging in...".dimmed());

                let login_result = self.client.login(&user, &password).await;

                match login_result {
                    Ok(login_response) => {
                        let creds = Credentials::with_refresh_token(
                            instance.clone(),
                            login_response.access_token,
                            login_response.user.id.to_string(),
                            login_response.expires_at.clone(),
                            Some(self.server_url.clone()),
                            login_response.refresh_token.clone(),
                            login_response.refresh_expires_at.clone(),
                        )
                        .with_identity_metadata(
                            login_response.user.name.clone(),
                            login_response.user.email.clone(),
                        );

                        store
                            .lock()
                            .expect("credential store lock should not be poisoned")
                            .set_credentials(&creds)?;

                        println!("{}", "✓ Credentials updated successfully".green().bold());
                        println!("  Instance: {}", instance.cyan());
                        let display_user = login_response
                            .user
                            .name
                            .clone()
                            .or_else(|| login_response.user.email.clone())
                            .unwrap_or_else(|| login_response.user.id.to_string());
                        println!("  User: {}", display_user.cyan());
                        println!("  Expires: {}", login_response.expires_at.cyan());
                        if let Some(ref refresh_expires) = login_response.refresh_expires_at {
                            println!("  Refresh expires: {}", refresh_expires.cyan());
                        }
                        println!("  Server URL: {}", self.server_url.cyan());
                        println!();
                        println!("{}", "Security Reminder:".yellow().bold());
                        println!(
                            "  Credentials are stored at: {}",
                            crate::credentials::FileCredentialStore::default_path()
                                .display()
                                .to_string()
                                .dimmed()
                        );
                        #[cfg(unix)]
                        println!("{}", "  File permissions: 0600 (owner read/write only)".dimmed());

                        Ok(())
                    },
                    Err(e) => Err(CLIError::ConfigurationError(format!("Login failed: {}", e))),
                }
            },
            (None, _) => Err(CLIError::ConfigurationError(
                "Instance name not set for this session".to_string(),
            )),
            (_, None) => Err(CLIError::ConfigurationError(
                "Credential store not initialized for this session".to_string(),
            )),
        }
    }

    pub(in crate::session) fn delete_credentials(&mut self) -> Result<()> {
        match (&self.instance, &mut self.credential_store) {
            (Some(instance), Some(store)) => {
                store
                    .lock()
                    .expect("credential store lock should not be poisoned")
                    .delete_credentials(instance)?;

                println!("{}", "✓ Credentials deleted successfully".green().bold());
                println!("  Instance: {}", instance.cyan());
                println!();
                println!(
                    "You will need to provide authentication credentials for future connections."
                );

                Ok(())
            },
            (None, _) => Err(CLIError::ConfigurationError(
                "Instance name not set for this session".to_string(),
            )),
            (_, None) => Err(CLIError::ConfigurationError(
                "Credential store not initialized for this session".to_string(),
            )),
        }
    }
}
