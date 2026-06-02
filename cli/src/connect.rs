use std::{io::IsTerminal, net::IpAddr, time::Duration};

use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use kalam_cli::{
    CLIConfiguration, CLIError, CLISession, FileCredentialStore, OutputFormat, Result,
};
use kalam_client::{
    credentials::{CredentialStore, Credentials},
    AuthProvider, KalamLinkClient, KalamLinkError, KalamLinkTimeouts, LoginResponse,
    ServerSetupRequest,
};
use url::Url;

use crate::args::Cli;
use crate::terminal_input::{prompt_line, prompt_password};

const DEFAULT_LOCAL_SERVER_URL: &str = "http://localhost:2900";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServerUrlSource {
    CliUrl,
    HostPort,
    StoredCredentials,
    DefaultLocalFallback,
}

fn preferred_user_label(user_id: &str, name: Option<&str>, email: Option<&str>) -> String {
    name
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            email
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| user_id.to_string())
}

fn credentials_from_login_response(
    instance: String,
    server_url: Option<String>,
    login_response: &LoginResponse,
) -> Credentials {
    Credentials::with_refresh_token(
        instance,
        login_response.access_token.clone(),
        login_response.user.id.to_string(),
        login_response.expires_at.clone(),
        server_url,
        login_response.refresh_token.clone(),
        login_response.refresh_expires_at.clone(),
    )
    .with_identity_metadata(
        login_response.user.name.clone(),
        login_response.user.email.clone(),
    )
}

impl ServerUrlSource {
    fn is_default_local_fallback(self) -> bool {
        matches!(self, Self::DefaultLocalFallback)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedServerUrl {
    value: String,
    source: ServerUrlSource,
}

/// Build timeouts configuration from CLI arguments
pub(crate) fn build_timeouts(cli: &Cli) -> KalamLinkTimeouts {
    // Check for preset flags first
    if cli.fast_timeouts {
        return KalamLinkTimeouts::fast();
    }
    if cli.relaxed_timeouts {
        return KalamLinkTimeouts::relaxed();
    }

    // Build custom timeouts from individual CLI args
    KalamLinkTimeouts::builder()
        .connection_timeout_secs(cli.connection_timeout)
        .receive_timeout_secs(cli.receive_timeout)
        .send_timeout_secs(cli.timeout) // Use general timeout for send
        .auth_timeout_secs(cli.auth_timeout)
        .subscribe_timeout_secs(5) // Keep default for subscribe ack
        .initial_data_timeout_secs(cli.initial_data_timeout)
        .idle_timeout_secs(cli.subscription_timeout) // subscription_timeout is the idle timeout
        .build()
}

fn is_localhost_url(url: &str) -> bool {
    let parsed = match Url::parse(url.trim()) {
        Ok(url) => url,
        Err(_) => return false,
    };

    let host = match parsed.host_str() {
        Some(host) => host,
        None => return false,
    };

    let normalized_host = host.trim_start_matches('[').trim_end_matches(']');

    if normalized_host.eq_ignore_ascii_case("localhost") {
        return true;
    }

    normalized_host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

fn should_default_to_http(raw: &str) -> bool {
    let candidate = format!("http://{}", raw.trim());
    let parsed = match Url::parse(&candidate) {
        Ok(url) => url,
        Err(_) => return false,
    };

    let host = match parsed.host_str() {
        Some(host) => host,
        None => return false,
    };

    let normalized_host = host.trim_start_matches('[').trim_end_matches(']');
    normalized_host.eq_ignore_ascii_case("localhost")
        || normalized_host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

pub(crate) fn normalize_and_validate_server_url(server_url: &str) -> Result<String> {
    let trimmed = server_url.trim();
    if trimmed.is_empty() {
        return Err(CLIError::ConfigurationError("Server URL must not be empty".to_string()));
    }

    let normalized_input = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        let scheme = if should_default_to_http(trimmed) {
            "http"
        } else {
            "https"
        };
        format!("{scheme}://{trimmed}")
    };

    let mut parsed = Url::parse(&normalized_input).map_err(|e| {
        CLIError::ConfigurationError(format!("Invalid server URL '{}': {}", server_url, e))
    })?;

    match parsed.scheme() {
        "http" | "https" => {},
        other => {
            return Err(CLIError::ConfigurationError(format!(
                "Unsupported URL scheme '{}'; expected http or https",
                other
            )));
        },
    }

    if parsed.host_str().is_none() {
        return Err(CLIError::ConfigurationError("Server URL must include a host".to_string()));
    }

    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(CLIError::ConfigurationError(
            "Server URL must not include embedded user/password credentials".to_string(),
        ));
    }

    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(CLIError::ConfigurationError(
            "Server URL must not include query parameters or fragments".to_string(),
        ));
    }

    parsed.set_query(None);
    parsed.set_fragment(None);

    let normalized = parsed.to_string();
    Ok(normalized.trim_end_matches('/').to_string())
}

pub(crate) fn resolve_server_url(
    cli: &Cli,
    credential_store: &FileCredentialStore,
) -> Result<String> {
    Ok(resolve_server_target(cli, credential_store)?.value)
}

fn resolve_server_target(
    cli: &Cli,
    credential_store: &FileCredentialStore,
) -> Result<ResolvedServerUrl> {
    let (server_url, source) = match (cli.url.clone(), cli.host.clone()) {
        (Some(url), _) => (url, ServerUrlSource::CliUrl),
        (None, Some(host)) => (format!("http://{}:{}", host, cli.port), ServerUrlSource::HostPort),
        (None, None) => {
            if let Some(creds) = credential_store.get_credentials(&cli.instance).map_err(|e| {
                CLIError::ConfigurationError(format!("Failed to load credentials: {}", e))
            })? {
                let creds_url = creds.get_server_url();
                if creds_url.starts_with("http://") || creds_url.starts_with("https://") {
                    (creds_url.to_string(), ServerUrlSource::StoredCredentials)
                } else {
                    (DEFAULT_LOCAL_SERVER_URL.to_string(), ServerUrlSource::DefaultLocalFallback)
                }
            } else {
                (DEFAULT_LOCAL_SERVER_URL.to_string(), ServerUrlSource::DefaultLocalFallback)
            }
        },
    };

    Ok(ResolvedServerUrl {
        value: normalize_and_validate_server_url(&server_url)?,
        source,
    })
}

fn render_login_banner(server_url: &str, source: ServerUrlSource, use_color: bool) -> Vec<String> {
    let title = " KALAMDB LOGIN ";
    let border = "=".repeat(title.len());
    let mut lines = Vec::with_capacity(9);

    lines.push(if use_color {
        border.bright_blue().bold().to_string()
    } else {
        border.clone()
    });
    lines.push(if use_color {
        title.white().bold().on_blue().to_string()
    } else {
        title.to_string()
    });
    lines.push(if use_color {
        border.bright_blue().bold().to_string()
    } else {
        border
    });

    let connection_message = match source {
        ServerUrlSource::DefaultLocalFallback => {
            "No server URL was configured. Kalam is using its default local address."
        },
        ServerUrlSource::StoredCredentials => "Using the server URL stored for this instance.",
        ServerUrlSource::CliUrl => "Using the server URL you provided.",
        ServerUrlSource::HostPort => "Using the host and port you provided.",
    };
    lines.push(if use_color && source.is_default_local_fallback() {
        connection_message.yellow().bold().to_string()
    } else if use_color {
        connection_message.bright_white().to_string()
    } else {
        connection_message.to_string()
    });

    let server_label = if use_color {
        "Server:".bright_cyan().bold().to_string()
    } else {
        "Server:".to_string()
    };
    let server_value = if use_color {
        server_url.cyan().bold().to_string()
    } else {
        server_url.to_string()
    };
    lines.push(format!("{} {}", server_label, server_value));

    if source.is_default_local_fallback() {
        let hint = "Tip: pass --url <server> to connect to a different KalamDB server.";
        lines.push(if use_color {
            hint.bright_black().to_string()
        } else {
            hint.to_string()
        });
    }

    if is_localhost_url(server_url) {
        let setup_note =
            "This local server is already initialized, so setup is not available here.";
        lines.push(if use_color {
            setup_note.yellow().to_string()
        } else {
            setup_note.to_string()
        });

        let account_note = "Use an existing KalamDB account to sign in.";
        lines.push(if use_color {
            account_note.bright_black().to_string()
        } else {
            account_note.to_string()
        });

        let cluster_note = "If this came from scripts/cluster.sh, sign in as 'root' with the configured root password (default: kalamdb123 unless changed).";
        lines.push(if use_color {
            cluster_note.bright_black().to_string()
        } else {
            cluster_note.to_string()
        });
    }

    let section_title = "Credentials";
    lines.push(if use_color {
        section_title.bright_white().bold().to_string()
    } else {
        section_title.to_string()
    });
    lines.push(if use_color {
        "-".repeat(section_title.len()).bright_black().to_string()
    } else {
        "-".repeat(section_title.len())
    });

    lines
}

fn render_prompt_label(label: &str, use_color: bool) -> String {
    if use_color {
        format!("{} ", label.bright_cyan().bold())
    } else {
        format!("{} ", label)
    }
}

fn create_connect_spinner(message: &str) -> ProgressBar {
    let progress_bar = ProgressBar::new_spinner();
    progress_bar.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.cyan} {msg} {elapsed_precise}")
            .expect("connect spinner template should be valid"),
    );
    progress_bar.set_message(message.to_string());
    progress_bar.enable_steady_tick(Duration::from_millis(80));
    progress_bar
}

pub async fn create_session(
    cli: &Cli,
    credential_store: &mut FileCredentialStore,
    config: &CLIConfiguration,
    config_path: std::path::PathBuf,
) -> Result<CLISession> {
    // Determine output format
    let format = if cli.json {
        OutputFormat::Json
    } else if cli.csv {
        OutputFormat::Csv
    } else {
        cli.format
    };

    // Determine server URL
    let server_target = resolve_server_target(cli, credential_store)?;
    let server_url_source = server_target.source;
    let server_url = server_target.value;

    if cli.verbose {
        eprintln!("Resolved server URL for instance '{}': {}", cli.instance, server_url);
    }

    fn simplify_login_error(err: &KalamLinkError) -> String {
        match err {
            KalamLinkError::AuthenticationError(message)
                if message.contains("Invalid credentials") =>
            {
                "invalid credentials".to_string()
            },
            _ => err.to_string(),
        }
    }

    /// Result of a login attempt
    enum LoginResult {
        Success(LoginResponse),
        SetupRequired,
        Failed(String),
        ConnectivityFailed(String),
    }

    // Helper function to exchange user/password for a JWT token
    async fn try_login(
        server_url: &str,
        user: &str,
        password: &str,
        verbose: bool,
        show_progress: bool,
    ) -> LoginResult {
        let login_spinner = if show_progress {
            Some(create_connect_spinner(&format!("Connecting to {}...", server_url)))
        } else {
            None
        };

        // Create a temporary client just for login (no auth needed for login endpoint)
        let temp_client = match KalamLinkClient::builder()
            .base_url(server_url)
            .timeout(Duration::from_secs(10))
            .build()
        {
            Ok(client) => client,
            Err(e) => {
                if let Some(pb) = login_spinner {
                    pb.finish_and_clear();
                }
                if verbose {
                    eprintln!("Warning: Could not create client for login: {}", e);
                }
                return LoginResult::Failed(e.to_string());
            },
        };

        match temp_client.login(user, password).await {
            Ok(response) => {
                if let Some(pb) = login_spinner {
                    pb.finish_with_message("Connected".to_string());
                }
                if verbose {
                    eprintln!(
                        "Successfully authenticated as '{}' (expires: {})",
                        response.user.id, response.expires_at
                    );
                }
                LoginResult::Success(response)
            },
            Err(KalamLinkError::SetupRequired(msg)) => {
                if let Some(pb) = login_spinner {
                    pb.finish_and_clear();
                }
                if verbose {
                    eprintln!("Server requires setup: {}", msg);
                }
                LoginResult::SetupRequired
            },
            Err(e) => {
                if let Some(pb) = login_spinner {
                    pb.finish_and_clear();
                }
                if verbose {
                    eprintln!("Warning: Login failed: {}", e);
                }
                if matches!(&e, KalamLinkError::NetworkError(_) | KalamLinkError::TimeoutError(_)) {
                    LoginResult::ConnectivityFailed(build_connectivity_diagnostics(server_url, &e))
                } else {
                    LoginResult::Failed(simplify_login_error(&e))
                }
            },
        }
    }

    /// Run the server setup wizard
    ///
    /// Returns the user and password that were set up so the caller can log in.
    async fn run_setup_wizard(server_url: &str) -> std::result::Result<(String, String), String> {
        println!();
        println!("╔═══════════════════════════════════════════════════════════════════╗");
        println!("║                    KalamDB Server Setup                           ║");
        println!("╠═══════════════════════════════════════════════════════════════════╣");
        println!("║                                                                   ║");
        println!("║  This server requires initial setup. You will need to:            ║");
        println!("║  1. Set a root password (for system administration)               ║");
        println!("║  2. Create your DBA user account                                  ║");
        println!("║                                                                   ║");
        println!("╚═══════════════════════════════════════════════════════════════════╝");
        println!();

        // Get DBA user
        let username =
            prompt_line("Enter the user for your DBA account: ").map_err(|e| e.to_string())?;
        if username.is_empty() {
            return Err("User cannot be empty".to_string());
        }
        if username.to_lowercase() == "root" {
            return Err("Cannot use 'root' as a user. Choose a different name.".to_string());
        }

        // Get DBA password
        let password =
            prompt_password("Enter password for your DBA account: ").map_err(|e| e.to_string())?;
        if password.is_empty() {
            return Err("Password cannot be empty".to_string());
        }

        // Confirm password
        let password_confirm = prompt_password("Confirm password: ").map_err(|e| e.to_string())?;
        if password != password_confirm {
            return Err("Passwords do not match".to_string());
        }

        // Get root password
        println!();
        println!("Now set the root password (for system administration):");
        let root_password = prompt_password("Enter root password: ").map_err(|e| e.to_string())?;
        if root_password.is_empty() {
            return Err("Root password cannot be empty".to_string());
        }

        // Confirm root password
        let root_password_confirm =
            prompt_password("Confirm root password: ").map_err(|e| e.to_string())?;
        if root_password != root_password_confirm {
            return Err("Root passwords do not match".to_string());
        }

        // Optional email
        let email = prompt_line("Enter email (optional, press Enter to skip): ")
            .map_err(|e| e.to_string())?;
        let email = if email.is_empty() { None } else { Some(email) };

        println!();
        println!("Setting up server...");

        // Create client and call setup endpoint
        let client = KalamLinkClient::builder()
            .base_url(server_url)
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| format!("Failed to create client: {}", e))?;

        let request =
            ServerSetupRequest::new(username.clone(), password.clone(), root_password, email);

        match client.server_setup(request).await {
            Ok(_response) => {
                println!();
                println!("╔═══════════════════════════════════════════════════════════════════╗");
                println!("║                    Setup Complete!                                ║");
                println!("╠═══════════════════════════════════════════════════════════════════╣");
                println!("║                                                                   ║");
                println!("║  ✓ Root password has been set                                     ║");
                println!(
                    "║  ✓ DBA user '{}' has been created{} ║",
                    username,
                    " ".repeat(36 - username.len().min(36))
                );
                println!("║                                                                   ║");
                println!("║  Please login with your new credentials.                          ║");
                println!("║                                                                   ║");
                println!("╚═══════════════════════════════════════════════════════════════════╝");
                println!();

                // Return the credentials so the caller can login
                Ok((username, password))
            },
            Err(e) => Err(format!("Setup failed: {}", e)),
        }
    }

    /// Perform setup and login with created credentials
    async fn setup_and_login(
        server_url: &str,
        verbose: bool,
        show_progress: bool,
        instance: &str,
        credential_store: &mut FileCredentialStore,
        save_credentials: bool,
    ) -> Result<(AuthProvider, Option<String>, bool)> {
        match run_setup_wizard(server_url).await {
            Ok((setup_username, setup_password)) => {
                match try_login(
                    server_url,
                    &setup_username,
                    &setup_password,
                    verbose,
                    show_progress,
                )
                .await
                {
                    LoginResult::Success(login_response) => {
                        let authenticated_user = login_response.user.id.to_string();

                        if save_credentials {
                            let new_creds = credentials_from_login_response(
                                instance.to_string(),
                                Some(server_url.to_string()),
                                &login_response,
                            );
                            let _ = credential_store.set_credentials(&new_creds);
                        }

                        Ok((
                            AuthProvider::jwt_token(login_response.access_token),
                            Some(preferred_user_label(
                                authenticated_user.as_str(),
                                login_response.user.name.as_deref(),
                                login_response.user.email.as_deref(),
                            )),
                            false,
                        ))
                    },
                    _ => Err(CLIError::SetupRequired(
                        "Setup completed but login failed. Please try logging in manually."
                            .to_string(),
                    )),
                }
            },
            Err(e) => Err(CLIError::SetupRequired(e)),
        }
    }

    /// Prompt user for credentials and attempt login (interactive only)
    async fn prompt_and_login(
        server_url: &str,
        server_url_source: ServerUrlSource,
        use_color: bool,
        verbose: bool,
        show_progress: bool,
        instance: &str,
        credential_store: &mut FileCredentialStore,
    ) -> Result<(AuthProvider, Option<String>, bool)> {
        // Before prompting for credentials, probe the server to detect whether it needs
        // initial setup. We use two methods:
        //
        // 1. check_setup_status() — works for localhost connections (server may block remote).
        // 2. A probe login as "root" with empty password — the server returns HTTP 428
        //    (SetupRequired) when root has no password, regardless of origin IP.
        //
        // Either detection redirects straight to the setup wizard without showing a username
        // prompt.
        let needs_setup = if let Ok(temp_client) = KalamLinkClient::builder()
            .base_url(server_url)
            .timeout(Duration::from_secs(10))
            .build()
        {
            // Try the status endpoint first (fast, no login attempt).
            let from_status = if let Ok(status) = temp_client.check_setup_status().await {
                status.needs_setup
            } else {
                false
            };

            // Fall back to the probe login if status endpoint didn't confirm.
            if from_status {
                true
            } else {
                matches!(
                    try_login(server_url, "root", "", verbose, show_progress).await,
                    LoginResult::SetupRequired
                )
            }
        } else {
            false
        };

        if needs_setup {
            return setup_and_login(
                server_url,
                verbose,
                show_progress,
                instance,
                credential_store,
                true,
            )
            .await;
        }

        println!();
        for line in render_login_banner(server_url, server_url_source, use_color) {
            println!("{}", line);
        }

        // Prompt for user
        let username = prompt_line(&render_prompt_label("User:", use_color))
            .map_err(|e| CLIError::FileError(format!("Failed to read user: {}", e)))?;

        if username.is_empty() {
            return Err(CLIError::ConfigurationError("User cannot be empty".to_string()));
        }

        // Prompt for password
        let password = prompt_password(&render_prompt_label("Password:", use_color))
            .map_err(|e| CLIError::FileError(format!("Failed to read password: {}", e)))?;

        // Try to login with provided credentials
        match try_login(server_url, &username, &password, verbose, show_progress).await {
            LoginResult::Success(login_response) => {
                let authenticated_user = login_response.user.id.to_string();

                // Ask if user wants to save credentials
                let save_choice =
                    prompt_line("\nSave credentials for future use? (y/N): ").unwrap_or_default();

                if save_choice.trim().eq_ignore_ascii_case("y")
                    || save_choice.trim().eq_ignore_ascii_case("yes")
                {
                    let new_creds = credentials_from_login_response(
                        instance.to_string(),
                        Some(server_url.to_string()),
                        &login_response,
                    );

                    if let Err(e) = credential_store.set_credentials(&new_creds) {
                        eprintln!("Warning: Could not save credentials: {}", e);
                    } else {
                        println!("Credentials saved for instance '{}'", instance);
                    }
                }

                println!();
                Ok((
                    AuthProvider::jwt_token(login_response.access_token),
                    Some(preferred_user_label(
                        authenticated_user.as_str(),
                        login_response.user.name.as_deref(),
                        login_response.user.email.as_deref(),
                    )),
                    false,
                ))
            },
            LoginResult::SetupRequired => {
                setup_and_login(
                    server_url,
                    verbose,
                    show_progress,
                    instance,
                    credential_store,
                    true,
                )
                .await
            },
            LoginResult::Failed(error) => {
                Err(CLIError::ConfigurationError(format!("Login failed: {}", error)))
            },
            LoginResult::ConnectivityFailed(error) => {
                Err(CLIError::LinkError(KalamLinkError::NetworkError(error)))
            },
        }
    }

    // Helper function to refresh access token using refresh token
    async fn try_refresh_token(
        server_url: &str,
        refresh_token: &str,
        verbose: bool,
    ) -> Option<LoginResponse> {
        let temp_client = match KalamLinkClient::builder()
            .base_url(server_url)
            .timeout(Duration::from_secs(10))
            .build()
        {
            Ok(client) => client,
            Err(e) => {
                if verbose {
                    eprintln!("Warning: Could not create client for token refresh: {}", e);
                }
                return None;
            },
        };

        match temp_client.refresh_access_token(refresh_token).await {
            Ok(response) => {
                if verbose {
                    eprintln!(
                        "Successfully refreshed token for '{}' (expires: {})",
                        response.user.id, response.expires_at
                    );
                }
                Some(response)
            },
            Err(e) => {
                if verbose {
                    eprintln!("Warning: Token refresh failed: {}", e);
                }
                None
            },
        }
    }

    // Determine authentication (priority: CLI args > stored credentials > localhost auto-auth)
    // Track: authenticated user, whether credentials were loaded from storage
    let (auth, authenticated_username, credentials_loaded) = if let Some(token) = cli
        .token
        .clone()
        .or_else(|| config.auth.as_ref().and_then(|a| a.jwt_token.clone()))
    {
        // Direct JWT token provided via --token or config - use it
        if cli.verbose {
            eprintln!("Using JWT token from CLI/config");
        }
        (AuthProvider::jwt_token(token), None, false)
    } else if let Some(username) = cli.user.clone() {
        // --user provided: login to get JWT token
        // If password is missing and terminal is available, prompt for it
        let password = if let Some(pwd) = cli.password.clone() {
            pwd
        } else if std::io::stdin().is_terminal() {
            println!();
            println!("User: {}", username);
            prompt_password("Password: ")
                .map_err(|e| CLIError::FileError(format!("Failed to read password: {}", e)))?
        } else {
            // Non-interactive mode without password - use empty password
            String::new()
        };

        match try_login(&server_url, &username, &password, cli.verbose, !cli.no_spinner).await {
            LoginResult::Success(login_response) => {
                let authenticated_user = login_response.user.id.to_string();

                // Only save credentials if --save-credentials flag is set
                if cli.save_credentials {
                    let new_creds = Credentials::with_refresh_token(
                        cli.instance.clone(),
                        login_response.access_token.clone(),
                        login_response.user.id.to_string(),
                        login_response.expires_at.clone(),
                        Some(server_url.clone()),
                        login_response.refresh_token.clone(),
                        login_response.refresh_expires_at.clone(),
                    );

                    if let Err(e) = credential_store.set_credentials(&new_creds) {
                        if cli.verbose {
                            eprintln!("Warning: Could not save credentials: {}", e);
                        }
                    } else if cli.verbose {
                        eprintln!("Saved JWT token for instance '{}'", cli.instance);
                    }
                }

                if cli.verbose {
                    eprintln!("Using JWT token for user '{}'", authenticated_user);
                }
                (
                    AuthProvider::jwt_token(login_response.access_token),
                    Some(authenticated_user),
                    false,
                )
            },
            LoginResult::SetupRequired => {
                setup_and_login(
                    &server_url,
                    cli.verbose,
                    !cli.no_spinner,
                    &cli.instance,
                    credential_store,
                    cli.save_credentials,
                )
                .await?
            },
            LoginResult::Failed(error) => {
                return Err(CLIError::ConfigurationError(format!("Login failed: {}", error)));
            },
            LoginResult::ConnectivityFailed(error) => {
                return Err(CLIError::LinkError(KalamLinkError::NetworkError(error)));
            },
        }
    } else if let Some(creds) = credential_store
        .get_credentials(&cli.instance)
        .map_err(|e| CLIError::ConfigurationError(format!("Failed to load credentials: {}", e)))?
    {
        // Load from stored credentials (JWT token)
        if creds.is_expired() {
            // Access token expired - try to refresh using refresh_token
            eprintln!("Warning: Stored credentials for '{}' have expired.", cli.instance);

            // Try to refresh the token if we have a valid refresh token
            if creds.can_refresh() {
                if cli.verbose {
                    eprintln!("Attempting to refresh access token...");
                }

                let refresh_server_url = creds.server_url.clone().unwrap_or(server_url.clone());
                if let Some(login_response) = try_refresh_token(
                    &refresh_server_url,
                    creds.refresh_token.as_ref().unwrap(),
                    cli.verbose,
                )
                .await
                {
                    // Save the refreshed credentials
                    let new_creds = credentials_from_login_response(
                        cli.instance.clone(),
                        Some(refresh_server_url),
                        &login_response,
                    );

                    if let Err(e) = credential_store.set_credentials(&new_creds) {
                        if cli.verbose {
                            eprintln!("Warning: Could not save refreshed credentials: {}", e);
                        }
                    } else {
                        eprintln!(
                            "Successfully refreshed credentials for instance '{}'",
                            cli.instance
                        );
                    }

                    let authenticated_user = login_response.user.id.to_string();
                    (
                        AuthProvider::jwt_token(login_response.access_token),
                        Some(preferred_user_label(
                            authenticated_user.as_str(),
                            login_response.user.name.as_deref(),
                            login_response.user.email.as_deref(),
                        )),
                        true,
                    )
                } else {
                    // Refresh failed - prompt for credentials if terminal is available
                    eprintln!("Warning: Could not refresh token.");

                    if std::io::stdin().is_terminal() {
                        // Prompt user for credentials interactively
                        prompt_and_login(
                            &server_url,
                            server_url_source,
                            !cli.no_color,
                            cli.verbose,
                            !cli.no_spinner,
                            &cli.instance,
                            credential_store,
                        )
                        .await?
                    } else if is_localhost_url(&server_url) {
                        // Non-interactive mode on localhost - try root auto-auth
                        let username = "root".to_string();
                        let password = "".to_string();

                        match try_login(
                            &server_url,
                            &username,
                            &password,
                            cli.verbose,
                            !cli.no_spinner,
                        )
                        .await
                        {
                            LoginResult::Success(login_response) => {
                                eprintln!("Auto-authenticated as root for localhost connection");
                                (
                                    AuthProvider::jwt_token(login_response.access_token),
                                    Some(login_response.user.id.to_string()),
                                    false,
                                )
                            },
                            LoginResult::SetupRequired => {
                                // Run setup wizard then login
                                match run_setup_wizard(&server_url).await {
                                    Ok((setup_username, setup_password)) => {
                                        match try_login(
                                            &server_url,
                                            &setup_username,
                                            &setup_password,
                                            cli.verbose,
                                            !cli.no_spinner,
                                        )
                                        .await
                                        {
                                            LoginResult::Success(login_response) => (
                                                AuthProvider::jwt_token(
                                                    login_response.access_token,
                                                ),
                                                Some(login_response.user.id.to_string()),
                                                false,
                                            ),
                                            _ => {
                                                return Err(CLIError::SetupRequired(
                                                    "Setup completed but login failed.".to_string(),
                                                ));
                                            },
                                        }
                                    },
                                    Err(e) => {
                                        return Err(CLIError::SetupRequired(e));
                                    },
                                }
                            },
                            LoginResult::Failed(_) | LoginResult::ConnectivityFailed(_) => {
                                (AuthProvider::None, None, false)
                            },
                        }
                    } else {
                        eprintln!(
                            "Please login again with --user and --password --save-credentials"
                        );
                        (AuthProvider::None, None, false)
                    }
                }
            } else {
                // No refresh token available - prompt for credentials if terminal is available
                eprintln!("Warning: No refresh token available.");

                if std::io::stdin().is_terminal() {
                    // Prompt user for credentials interactively
                    prompt_and_login(
                        &server_url,
                        server_url_source,
                        !cli.no_color,
                        cli.verbose,
                        !cli.no_spinner,
                        &cli.instance,
                        credential_store,
                    )
                    .await?
                } else if is_localhost_url(&server_url) {
                    // Non-interactive mode on localhost - try root auto-auth
                    let username = "root".to_string();
                    let password = "".to_string();

                    match try_login(&server_url, &username, &password, cli.verbose, !cli.no_spinner)
                        .await
                    {
                        LoginResult::Success(login_response) => {
                            eprintln!("Auto-authenticated as root for localhost connection");
                            (
                                AuthProvider::jwt_token(login_response.access_token),
                                Some(login_response.user.id.to_string()),
                                false,
                            )
                        },
                        LoginResult::SetupRequired => {
                            // Run setup wizard then login
                            match run_setup_wizard(&server_url).await {
                                Ok((setup_username, setup_password)) => {
                                    match try_login(
                                        &server_url,
                                        &setup_username,
                                        &setup_password,
                                        cli.verbose,
                                        !cli.no_spinner,
                                    )
                                    .await
                                    {
                                        LoginResult::Success(login_response) => (
                                            AuthProvider::jwt_token(login_response.access_token),
                                            Some(login_response.user.id.to_string()),
                                            false,
                                        ),
                                        _ => {
                                            return Err(CLIError::SetupRequired(
                                                "Setup completed but login failed.".to_string(),
                                            ));
                                        },
                                    }
                                },
                                Err(e) => {
                                    return Err(CLIError::SetupRequired(e));
                                },
                            }
                        },
                        LoginResult::Failed(_) | LoginResult::ConnectivityFailed(_) => {
                            (AuthProvider::None, None, false)
                        },
                    }
                } else {
                    eprintln!("Please login again with --user and --password --save-credentials");
                    (AuthProvider::None, None, false)
                }
            }
        } else {
            // Token is still valid
            let stored_username = creds.display_label().map(str::to_string);
            if cli.verbose {
                if let Some(ref user) = stored_username {
                    eprintln!(
                        "Using stored JWT token for user '{}' (instance: {})",
                        user, cli.instance
                    );
                } else {
                    eprintln!("Using stored JWT token for instance '{}'", cli.instance);
                }
            }
            (AuthProvider::jwt_token(creds.jwt_token), stored_username, true)
        }
    } else {
        // No credentials provided - prompt interactively if terminal is available
        if std::io::stdin().is_terminal() {
            prompt_and_login(
                &server_url,
                server_url_source,
                !cli.no_color,
                cli.verbose,
                !cli.no_spinner,
                &cli.instance,
                credential_store,
            )
            .await?
        } else if is_localhost_url(&server_url) {
            // Non-interactive mode on localhost - try root auto-auth
            let username = "root".to_string();
            let password = "".to_string();

            match try_login(&server_url, &username, &password, cli.verbose, !cli.no_spinner).await {
                LoginResult::Success(login_response) => {
                    if cli.verbose {
                        eprintln!("Auto-authenticated as root for localhost connection");
                    }
                    (
                        AuthProvider::jwt_token(login_response.access_token),
                        Some(login_response.user.id.to_string()),
                        false,
                    )
                },
                LoginResult::SetupRequired => {
                    // Server requires initial setup - run the setup wizard then login
                    match run_setup_wizard(&server_url).await {
                        Ok((setup_username, setup_password)) => {
                            match try_login(
                                &server_url,
                                &setup_username,
                                &setup_password,
                                cli.verbose,
                                !cli.no_spinner,
                            )
                            .await
                            {
                                LoginResult::Success(login_response) => {
                                    let authenticated_user = login_response.user.id.to_string();

                                    // Save credentials after successful setup
                                    let new_creds = credentials_from_login_response(
                                        cli.instance.clone(),
                                        Some(server_url.clone()),
                                        &login_response,
                                    );
                                    let _ = credential_store.set_credentials(&new_creds);

                                    (
                                        AuthProvider::jwt_token(login_response.access_token),
                                        Some(authenticated_user),
                                        false,
                                    )
                                },
                                _ => {
                                    return Err(CLIError::SetupRequired(
                                        "Setup completed but login failed. Please try logging in \
                                         manually."
                                            .to_string(),
                                    ));
                                },
                            }
                        },
                        Err(e) => {
                            return Err(CLIError::SetupRequired(e));
                        },
                    }
                },
                LoginResult::Failed(e) | LoginResult::ConnectivityFailed(e) => {
                    if cli.verbose {
                        eprintln!("Auto-login failed: {}", e);
                    }
                    (AuthProvider::None, None, false)
                },
            }
        } else {
            // Non-interactive mode and not localhost - no auth available
            return Err(CLIError::ConfigurationError(
                "No authentication credentials available. Use --user and --password, or run \
                 interactively."
                    .to_string(),
            ));
        }
    };

    let mut connection_options = config.to_connection_options();
    if server_url.starts_with("http://") {
        use kalam_client::HttpVersion;
        if connection_options.http_version == HttpVersion::Http2 {
            connection_options = connection_options.with_http_version(HttpVersion::Auto);
        }
    }

    fn build_connectivity_diagnostics(server_url: &str, err: &KalamLinkError) -> String {
        let error_details = match err {
            KalamLinkError::NetworkError(msg) | KalamLinkError::TimeoutError(msg) => msg.as_str(),
            _ => &err.to_string(),
        };

        format!(
            "Connection failed: {}\n\nPossible issues:\n  • Server is not running on {}\n  • URL/port is incorrect\n  • Network connectivity issue\n\nTry:\n  • Check if server is running: curl {}/v1/api/healthcheck\n  • Verify the server URL with --url (example: http://127.0.0.1:2900)",
            error_details, server_url, server_url
        )
    }

    // Test auth by creating a test client and trying to execute a simple query
    let test_auth_result = {
        let timeouts = build_timeouts(cli);
        let test_builder = KalamLinkClient::builder()
            .base_url(&server_url)
            .timeout(Duration::from_secs(cli.timeout))
            .auth(auth.clone())
            .timeouts(timeouts)
            .connection_options(connection_options.clone());

        let test_client = test_builder.build()?;
        // Verify authentication without requiring system table privileges.
        test_client.execute_query("SELECT 1 AS auth_check", None, None, None).await
    };

    // Check if auth test failed
    let session_result = match test_auth_result {
        Ok(_) => {
            // Auth works, create session normally
            CLISession::with_auth_and_instance(
                server_url.clone(),
                auth.clone(),
                format,
                !cli.no_color,
                Some(cli.instance.clone()),
                Some(credential_store.clone()),
                authenticated_username,
                cli.loading_threshold_ms,
                !cli.no_spinner,
                Some(Duration::from_secs(cli.timeout)),
                Some(build_timeouts(cli)),
                Some(connection_options.clone()),
                config.clone(),
                config_path.clone(),
                credentials_loaded,
            )
            .await
        },
        Err(e) => {
            // If the server is unreachable, return detailed diagnostics.
            if matches!(e, KalamLinkError::NetworkError(_) | KalamLinkError::TimeoutError(_)) {
                Err(CLIError::LinkError(KalamLinkError::NetworkError(
                    build_connectivity_diagnostics(&server_url, &e),
                )))
            } else {
                // Auth test failed - return as error for handling below
                Err(CLIError::LinkError(e))
            }
        },
    };

    // If session creation failed with an auth error and no --user was provided, prompt for login
    match session_result {
        Ok(session) => Ok(session),
        Err(ref e) => {
            // Check if setup is required first
            let is_setup_required =
                matches!(e, CLIError::LinkError(KalamLinkError::SetupRequired(_)));

            // Check if it's an auth-related error
            let is_auth_error = match e {
                CLIError::LinkError(KalamLinkError::ServerError { status_code, .. }) => {
                    *status_code == 401
                },
                CLIError::LinkError(KalamLinkError::AuthenticationError(_)) => true,
                _ => false,
            };

            // If setup is required and terminal is interactive, run setup wizard directly
            if is_setup_required && std::io::stdin().is_terminal() {
                eprintln!("\n{}", "Server requires initial setup.".yellow().bold());

                // Run setup wizard
                match setup_and_login(
                    &server_url,
                    cli.verbose,
                    !cli.no_spinner,
                    &cli.instance,
                    credential_store,
                    cli.save_credentials,
                )
                .await
                {
                    Ok((new_auth, new_username, new_creds_loaded)) => {
                        // Create session with new credentials from setup
                        CLISession::with_auth_and_instance(
                            server_url,
                            new_auth,
                            format,
                            !cli.no_color,
                            Some(cli.instance.clone()),
                            Some(credential_store.clone()),
                            new_username,
                            cli.loading_threshold_ms,
                            !cli.no_spinner,
                            Some(Duration::from_secs(cli.timeout)),
                            Some(build_timeouts(cli)),
                            Some(connection_options),
                            config.clone(),
                            config_path,
                            new_creds_loaded,
                        )
                        .await
                    },
                    Err(setup_err) => Err(setup_err),
                }
            } else if is_auth_error
                && cli.user.is_none()
                && cli.token.is_none()
                && std::io::stdin().is_terminal()
            {
                // Auth failure detected (not setup) and we can prompt for new credentials
                eprintln!("\n{}", "Authentication failed with stored credentials.".yellow().bold());

                // Prompt for new credentials
                let (new_auth, new_username, new_creds_loaded) = prompt_and_login(
                    &server_url,
                    server_url_source,
                    !cli.no_color,
                    cli.verbose,
                    !cli.no_spinner,
                    &cli.instance,
                    credential_store,
                )
                .await?;

                // Retry session creation with new credentials
                CLISession::with_auth_and_instance(
                    server_url,
                    new_auth,
                    format,
                    !cli.no_color,
                    Some(cli.instance.clone()),
                    Some(credential_store.clone()),
                    new_username,
                    cli.loading_threshold_ms,
                    !cli.no_spinner,
                    Some(Duration::from_secs(cli.timeout)),
                    Some(build_timeouts(cli)),
                    Some(connection_options),
                    config.clone(),
                    config_path,
                    new_creds_loaded,
                )
                .await
            } else {
                // Non-interactive or CLI args provided - return the original error
                session_result
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use std::{
        env,
        time::{SystemTime, UNIX_EPOCH},
    };

    use clap::Parser;

    use super::*;
    use crate::args::Cli;
    use kalam_cli::FileCredentialStore;
    use kalam_client::KalamLinkTimeouts;

    #[test]
    fn test_is_localhost_url_accepts_loopback_hosts() {
        assert!(is_localhost_url("http://localhost:2900"));
        assert!(is_localhost_url("https://127.0.0.1:8443"));
        assert!(is_localhost_url("http://[::1]:2900"));
        assert!(!is_localhost_url("http://0.0.0.0:2900"));
    }

    #[test]
    fn test_is_localhost_url_rejects_spoofed_hosts() {
        assert!(!is_localhost_url("http://localhost.evil.com:2900"));
        assert!(!is_localhost_url("http://evil-localhost.example:2900"));
        assert!(!is_localhost_url("http://example.com/?target=localhost"));
        assert!(!is_localhost_url("not-a-url"));
    }

    #[test]
    fn test_normalize_and_validate_server_url_accepts_http_https() {
        assert_eq!(
            normalize_and_validate_server_url("http://localhost:2900/").unwrap(),
            "http://localhost:2900"
        );
        assert_eq!(
            normalize_and_validate_server_url("https://db.example.com/base/").unwrap(),
            "https://db.example.com/base"
        );
        assert_eq!(
            normalize_and_validate_server_url("kalam.masky.app").unwrap(),
            "https://kalam.masky.app"
        );
        assert_eq!(
            normalize_and_validate_server_url("localhost:2900").unwrap(),
            "http://localhost:2900"
        );
    }

    #[test]
    fn test_normalize_and_validate_server_url_rejects_sensitive_or_invalid_parts() {
        assert!(normalize_and_validate_server_url("ftp://localhost:2900").is_err());
        assert!(normalize_and_validate_server_url("http://user:pass@localhost:2900").is_err());
        assert!(normalize_and_validate_server_url("http://localhost:2900?token=secret").is_err());
        assert!(normalize_and_validate_server_url("http://localhost:2900/#fragment").is_err());
    }

    #[test]
    fn test_build_timeouts_preserves_sdk_keepalive_default() {
        let cli = Cli::parse_from(["kalam"]);

        let timeouts = build_timeouts(&cli);

        assert_eq!(timeouts.keepalive_interval, KalamLinkTimeouts::default().keepalive_interval);
    }

    #[test]
    fn test_resolve_server_target_defaults_to_localhost_when_unconfigured() {
        let cli = Cli::parse_from(["kalam"]);
        let unique_suffix = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let temp_path = env::temp_dir().join(format!(
            "kalam-connect-test-{}-{}.toml",
            std::process::id(),
            unique_suffix
        ));
        let store = FileCredentialStore::with_path(temp_path).unwrap();

        let resolved = resolve_server_target(&cli, &store).unwrap();

        assert_eq!(resolved.value, DEFAULT_LOCAL_SERVER_URL);
        assert_eq!(resolved.source, ServerUrlSource::DefaultLocalFallback);
    }

    #[test]
    fn test_render_login_banner_explains_default_local_fallback() {
        let rendered = render_login_banner(
            DEFAULT_LOCAL_SERVER_URL,
            ServerUrlSource::DefaultLocalFallback,
            false,
        )
        .join("\n");

        assert!(rendered
            .contains("No server URL was configured. Kalam is using its default local address."));
        assert!(
            rendered.contains("Tip: pass --url <server> to connect to a different KalamDB server.")
        );
        assert!(rendered.contains("If this came from scripts/cluster.sh, sign in as 'root'"));
        assert!(!rendered.contains("Please enter your credentials to connect to:"));
    }
}
