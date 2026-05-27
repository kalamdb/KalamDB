use colored::Colorize;

pub fn print_oidc_browser_prompt(provider: &str, authorization_url: &str, color: bool) {
    print_auth_header("OIDC browser login", color);
    println!("Provider: {}", style_value(provider, color));
    println!("Open this URL to continue:");
    println!("{}", terminal_hyperlink(authorization_url, color));
    println!();
}

pub fn print_oidc_device_prompt(
    provider: &str,
    verification_uri: &str,
    complete_uri: Option<&str>,
    user_code: &str,
    color: bool,
) {
    print_auth_header("OIDC device login", color);
    println!("Provider: {}", style_value(provider, color));
    println!("Open this URL on any device:");
    println!("{}", terminal_hyperlink(complete_uri.unwrap_or(verification_uri), color));
    println!("User code: {}", style_value(user_code, color));
    println!();
}

fn print_auth_header(title: &str, color: bool) {
    let brand = if color {
        "KalamDB CLI".bright_blue().bold().to_string()
    } else {
        "KalamDB CLI".to_string()
    };
    let title = style_value(title, color);
    println!("{brand} - {title}");
}

fn style_value(value: &str, color: bool) -> String {
    if color {
        value.bright_cyan().bold().to_string()
    } else {
        value.to_string()
    }
}

fn terminal_hyperlink(url: &str, color: bool) -> String {
    if !color {
        return url.to_string();
    }
    format!("\x1b]8;;{url}\x1b\\{}\x1b]8;;\x1b\\", url.underline())
}
