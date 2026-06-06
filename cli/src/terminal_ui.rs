use std::{
    io::{self, Write},
    time::Duration,
};

use colored::Colorize;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    terminal::{self, ClearType},
    ExecutableCommand,
};
use indicatif::{ProgressBar, ProgressStyle};

const SPINNER_TICKS: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const SPINNER_TICK_INTERVAL_MS: u64 = 80;

#[derive(Debug, Clone, Copy)]
pub struct SelectOption<'a> {
    pub label: &'a str,
    pub description: Option<&'a str>,
}

impl<'a> SelectOption<'a> {
    pub const fn new(label: &'a str) -> Self {
        Self {
            label,
            description: None,
        }
    }

    pub const fn described(label: &'a str, description: &'a str) -> Self {
        Self {
            label,
            description: Some(description),
        }
    }
}

pub fn print_banner(title: &str, subtitle: Option<&str>, color: bool) {
    for line in banner_lines(title, subtitle, color) {
        println!("{line}");
    }
}

pub fn banner_lines(title: &str, subtitle: Option<&str>, color: bool) -> Vec<String> {
    let normalized_title = format!(" {} ", title.trim().to_ascii_uppercase());
    let border = "=".repeat(normalized_title.len());
    let mut lines = Vec::with_capacity(if subtitle.is_some() { 4 } else { 3 });

    lines.push(style_accent(&border, color));
    lines.push(if color {
        normalized_title.white().bold().on_blue().to_string()
    } else {
        normalized_title
    });
    lines.push(style_accent(&border, color));

    if let Some(subtitle) = subtitle {
        lines.push(style_info(subtitle, color));
    }

    lines
}

pub fn prompt_label(label: &str, color: bool) -> String {
    let label = label.trim_end();
    if color {
        format!("{} ", label.bright_cyan().bold())
    } else {
        format!("{label} ")
    }
}

pub fn prompt_label_with_default(label: &str, default: &str, color: bool) -> String {
    let label = label.trim_end_matches(':').trim_end();
    let text = format!("{label} [{default}]:");
    prompt_label(&text, color)
}

pub fn style_value(value: &str, color: bool) -> String {
    if color {
        value.bright_cyan().bold().to_string()
    } else {
        value.to_string()
    }
}

pub fn style_success(value: &str, color: bool) -> String {
    if color {
        value.green().to_string()
    } else {
        value.to_string()
    }
}

pub fn style_warning(value: &str, color: bool) -> String {
    if color {
        value.yellow().bold().to_string()
    } else {
        value.to_string()
    }
}

pub fn style_muted(value: &str, color: bool) -> String {
    if color {
        value.bright_black().to_string()
    } else {
        value.to_string()
    }
}

pub fn style_info(value: &str, color: bool) -> String {
    if color {
        value.bright_white().to_string()
    } else {
        value.to_string()
    }
}

pub fn style_accent(value: &str, color: bool) -> String {
    if color {
        value.bright_blue().bold().to_string()
    } else {
        value.to_string()
    }
}

pub fn create_spinner(message: &str) -> ProgressBar {
    create_spinner_inner(message, false)
}

pub fn create_spinner_with_elapsed(message: &str) -> ProgressBar {
    create_spinner_inner(message, true)
}

fn create_spinner_inner(message: &str, elapsed: bool) -> ProgressBar {
    let progress_bar = ProgressBar::new_spinner();
    let template = if elapsed {
        "{spinner:.cyan} {msg} {elapsed_precise}"
    } else {
        "{spinner:.cyan} {msg}"
    };
    progress_bar.set_style(
        ProgressStyle::default_spinner()
            .tick_strings(SPINNER_TICKS)
            .template(template)
            .expect("spinner template should be valid"),
    );
    progress_bar.set_message(message.to_string());
    progress_bar.enable_steady_tick(Duration::from_millis(SPINNER_TICK_INTERVAL_MS));
    progress_bar
}

pub fn prompt_select(
    title: &str,
    options: &[SelectOption<'_>],
    default_index: usize,
    color: bool,
) -> io::Result<usize> {
    if options.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "prompt_select requires at least one option",
        ));
    }

    let mut selected = default_index.min(options.len() - 1);
    run_menu(|stdout, rendered_lines| loop {
        redraw_menu(stdout, *rendered_lines, |stdout| {
            render_select_menu(stdout, title, options, selected, color)
        })?;
        *rendered_lines = menu_line_count(options.len(), false);

        match read_menu_key()? {
            MenuKey::Up => selected = selected.saturating_sub(1),
            MenuKey::Down => {
                if selected + 1 < options.len() {
                    selected += 1;
                }
            },
            MenuKey::Confirm => return Ok(selected),
            MenuKey::Cancel => {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "setup cancelled"));
            },
            MenuKey::Toggle | MenuKey::Other => {},
        }
    })
}

pub fn prompt_multi_select(
    title: &str,
    options: &[SelectOption<'_>],
    default_selected: &[bool],
    color: bool,
) -> io::Result<Vec<usize>> {
    if options.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "prompt_multi_select requires at least one option",
        ));
    }

    let mut selected: Vec<bool> = options
        .iter()
        .enumerate()
        .map(|(idx, _)| default_selected.get(idx).copied().unwrap_or(false))
        .collect();
    if !selected.iter().any(|value| *value) {
        selected[0] = true;
    }

    let mut cursor_index = 0usize;
    run_menu(|stdout, rendered_lines| loop {
        redraw_menu(stdout, *rendered_lines, |stdout| {
            render_multi_select_menu(stdout, title, options, &selected, cursor_index, color)
        })?;
        *rendered_lines = menu_line_count(options.len(), true);

        match read_menu_key()? {
            MenuKey::Up => cursor_index = cursor_index.saturating_sub(1),
            MenuKey::Down => {
                if cursor_index + 1 < options.len() {
                    cursor_index += 1;
                }
            },
            MenuKey::Toggle => {
                selected[cursor_index] = !selected[cursor_index];
                if !selected.iter().any(|value| *value) {
                    selected[cursor_index] = true;
                }
            },
            MenuKey::Confirm => {
                return Ok(selected
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, checked)| checked.then_some(idx))
                    .collect());
            },
            MenuKey::Cancel => {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "setup cancelled"));
            },
            MenuKey::Other => {},
        }
    })
}

pub fn print_oidc_browser_prompt(provider: &str, authorization_url: &str, color: bool) {
    print_banner("OIDC browser login", Some("Authenticate with your identity provider."), color);
    println!("{} {}", prompt_label("Provider:", color), style_value(provider, color));
    println!("{}", style_info("Open this URL to continue:", color));
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
    print_banner("OIDC device login", Some("Authenticate from any device."), color);
    println!("{} {}", prompt_label("Provider:", color), style_value(provider, color));
    println!("{}", style_info("Open this URL on any device:", color));
    println!("{}", terminal_hyperlink(complete_uri.unwrap_or(verification_uri), color));
    println!("{} {}", prompt_label("User code:", color), style_value(user_code, color));
    println!();
}

fn run_menu<T>(mut f: impl FnMut(&mut io::Stdout, &mut usize) -> io::Result<T>) -> io::Result<T> {
    let mut stdout = io::stdout();
    terminal::enable_raw_mode()?;
    stdout.execute(cursor::Hide)?;

    let mut rendered_lines = 0usize;
    let result = f(&mut stdout, &mut rendered_lines);
    let clear_result = clear_rendered_menu(&mut stdout, rendered_lines);
    let _ = stdout.execute(cursor::Show);
    let _ = terminal::disable_raw_mode();

    match (result, clear_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}

fn redraw_menu(
    stdout: &mut io::Stdout,
    rendered_lines: usize,
    render: impl FnOnce(&mut io::Stdout) -> io::Result<()>,
) -> io::Result<()> {
    clear_rendered_menu(stdout, rendered_lines)?;
    render(stdout)?;
    stdout.flush()
}

fn clear_rendered_menu(stdout: &mut io::Stdout, rendered_lines: usize) -> io::Result<()> {
    for _ in 0..rendered_lines {
        stdout.execute(cursor::MoveUp(1))?;
        stdout.execute(cursor::MoveToColumn(0))?;
        stdout.execute(terminal::Clear(ClearType::CurrentLine))?;
    }
    stdout.flush()
}

fn render_select_menu(
    stdout: &mut io::Stdout,
    title: &str,
    options: &[SelectOption<'_>],
    selected: usize,
    color: bool,
) -> io::Result<()> {
    writeln!(stdout, "{}", style_accent(title, color))?;
    writeln!(stdout)?;
    for (idx, option) in options.iter().enumerate() {
        let cursor = if idx == selected { ">" } else { " " };
        let radio = if idx == selected { "(*)" } else { "( )" };
        let label = if idx == selected {
            style_value(option.label, color)
        } else {
            option.label.to_string()
        };
        write!(stdout, "  {cursor} {radio} {label}")?;
        if let Some(description) = option.description {
            write!(stdout, "  {}", style_muted(description, color))?;
        }
        writeln!(stdout)?;
    }
    writeln!(stdout)?;
    writeln!(stdout, "{}", style_muted("Up/Down move, Enter confirm, Esc cancel", color))
}

fn render_multi_select_menu(
    stdout: &mut io::Stdout,
    title: &str,
    options: &[SelectOption<'_>],
    selected: &[bool],
    cursor_index: usize,
    color: bool,
) -> io::Result<()> {
    writeln!(stdout, "{}", style_accent(title, color))?;
    writeln!(stdout)?;
    for (idx, option) in options.iter().enumerate() {
        let cursor = if idx == cursor_index { ">" } else { " " };
        let checkbox = if selected[idx] { "[x]" } else { "[ ]" };
        let label = if idx == cursor_index {
            style_value(option.label, color)
        } else {
            option.label.to_string()
        };
        write!(stdout, "  {cursor} {checkbox} {label}")?;
        if let Some(description) = option.description {
            write!(stdout, "  {}", style_muted(description, color))?;
        }
        writeln!(stdout)?;
    }
    writeln!(stdout)?;
    writeln!(
        stdout,
        "{}",
        style_muted("Up/Down move, Space toggle, Enter confirm, Esc cancel", color)
    )
}

fn menu_line_count(option_count: usize, _multi: bool) -> usize {
    // title + blank + options + blank + controls
    option_count + 4
}

enum MenuKey {
    Up,
    Down,
    Toggle,
    Confirm,
    Cancel,
    Other,
}

fn read_menu_key() -> io::Result<MenuKey> {
    loop {
        if !event::poll(Duration::from_millis(100))? {
            continue;
        }

        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                return Ok(match key.code {
                    KeyCode::Up | KeyCode::Char('k') => MenuKey::Up,
                    KeyCode::Down | KeyCode::Char('j') => MenuKey::Down,
                    KeyCode::Char(' ') => MenuKey::Toggle,
                    KeyCode::Enter => MenuKey::Confirm,
                    KeyCode::Esc => MenuKey::Cancel,
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        MenuKey::Cancel
                    },
                    _ => MenuKey::Other,
                });
            },
            _ => {},
        }
    }
}

fn terminal_hyperlink(url: &str, color: bool) -> String {
    if !color {
        return url.to_string();
    }
    format!("\x1b]8;;{url}\x1b\\{}\x1b]8;;\x1b\\", url.underline())
}
