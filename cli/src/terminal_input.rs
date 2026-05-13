use std::io::{self, Write};

use crossterm::terminal;

struct CookedTerminalGuard {
    restore_raw_mode: bool,
}

impl CookedTerminalGuard {
    fn acquire() -> io::Result<Self> {
        let restore_raw_mode = terminal::is_raw_mode_enabled()?;
        if restore_raw_mode {
            terminal::disable_raw_mode()?;
        }

        Ok(Self { restore_raw_mode })
    }
}

impl Drop for CookedTerminalGuard {
    fn drop(&mut self) {
        if self.restore_raw_mode {
            let _ = terminal::enable_raw_mode();
        }
    }
}

pub fn prompt_line(prompt: &str) -> io::Result<String> {
    let _guard = CookedTerminalGuard::acquire()?;

    print!("{prompt}");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    Ok(input.trim().to_string())
}

pub fn prompt_password(prompt: &str) -> io::Result<String> {
    let _guard = CookedTerminalGuard::acquire()?;
    rpassword::prompt_password(prompt)
}