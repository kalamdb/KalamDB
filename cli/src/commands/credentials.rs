use kalam_cli::{workflow::project::preferred_user_label, CLIError, FileCredentialStore, Result};
use kalam_client::credentials::CredentialStore;

use crate::args::Cli;

pub fn handle_credentials(cli: &Cli, credential_store: &mut FileCredentialStore) -> Result<bool> {
    if !cli.list_instances {
        return Ok(false);
    }

    let instances = credential_store
        .list_instances()
        .map_err(|e| CLIError::ConfigurationError(format!("Failed to list instances: {}", e)))?;
    if instances.is_empty() {
        println!("No stored credentials");
    } else {
        println!("Stored credential instances:");
        for instance in instances {
            if let Ok(Some(creds)) = credential_store.get_credentials(&instance) {
                let user_info = creds
                    .user
                    .as_ref()
                    .map(|user| {
                        preferred_user_label(user, creds.name.as_deref(), creds.email.as_deref())
                    })
                    .or_else(|| creds.display_label().map(str::to_string))
                    .unwrap_or_else(|| "unknown".to_string());
                let expired = if creds.is_expired() { " (expired)" } else { "" };
                println!("  • {} (user: {}){}", instance, user_info, expired);
            } else {
                println!("  • {}", instance);
            }
        }
    }
    Ok(true)
}
