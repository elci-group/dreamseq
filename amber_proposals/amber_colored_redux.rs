//! amber_color — Minimal replacement for terminal coloring crates
//!
//! Supports basic ANSI color codes without external dependencies.

pub trait Colorize {
    fn red(self) -> String;
    fn green(self) -> String;
    fn yellow(self) -> String;
    fn blue(self) -> String;
    fn magenta(self) -> String;
    fn cyan(self) -> String;
    fn bold(self) -> String;
    fn dimmed(self) -> String;
    fn underline(self) -> String;
}

impl Colorize for &str {
    fn red(self) -> String { format!("\x1b[31m{}\x1b[0m", self) }
    fn green(self) -> String { format!("\x1b[32m{}\x1b[0m", self) }
    fn yellow(self) -> String { format!("\x1b[33m{}\x1b[0m", self) }
    fn blue(self) -> String { format!("\x1b[34m{}\x1b[0m", self) }
    fn magenta(self) -> String { format!("\x1b[35m{}\x1b[0m", self) }
    fn cyan(self) -> String { format!("\x1b[36m{}\x1b[0m", self) }
    fn bold(self) -> String { format!("\x1b[1m{}\x1b[0m", self) }
    fn dimmed(self) -> String { format!("\x1b[2m{}\x1b[0m", self) }
    fn underline(self) -> String { format!("\x1b[4m{}\x1b[0m", self) }
}

impl Colorize for String {
    fn red(self) -> String { format!("\x1b[31m{}\x1b[0m", self) }
    fn green(self) -> String { format!("\x1b[32m{}\x1b[0m", self) }
    fn yellow(self) -> String { format!("\x1b[33m{}\x1b[0m", self) }
    fn blue(self) -> String { format!("\x1b[34m{}\x1b[0m", self) }
    fn magenta(self) -> String { format!("\x1b[35m{}\x1b[0m", self) }
    fn cyan(self) -> String { format!("\x1b[36m{}\x1b[0m", self) }
    fn bold(self) -> String { format!("\x1b[1m{}\x1b[0m", self) }
    fn dimmed(self) -> String { format!("\x1b[2m{}\x1b[0m", self) }
    fn underline(self) -> String { format!("\x1b[4m{}\x1b[0m", self) }
}

/// StyledString for building colored output
pub struct StyledString(pub String);

impl StyledString {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn push_colored(&mut self, s: &str, color: &str) {
        let colored = match color {
            "red" => s.red(),
            "green" => s.green(),
            "yellow" => s.yellow(),
            "blue" => s.blue(),
            "cyan" => s.cyan(),
            "magenta" => s.magenta(),
            _ => s.to_string(),
        };
        self.0.push_str(&colored);
    }
}

