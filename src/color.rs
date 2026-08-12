//! Minimal terminal styling used by the human-readable renderer.

const RESET: &str = "\x1b[0m";

pub trait Colorize {
    fn red(&self) -> String;
    fn green(&self) -> String;
    fn yellow(&self) -> String;
    fn blue(&self) -> String;
    fn cyan(&self) -> String;
    fn bold(&self) -> String;
    fn dimmed(&self) -> String;
    fn white(&self) -> String;
}

fn style(value: &str, code: &str) -> String {
    format!("\x1b[{code}m{value}{RESET}")
}

impl Colorize for str {
    fn red(&self) -> String {
        style(self, "31")
    }
    fn green(&self) -> String {
        style(self, "32")
    }
    fn yellow(&self) -> String {
        style(self, "33")
    }
    fn blue(&self) -> String {
        style(self, "34")
    }
    fn cyan(&self) -> String {
        style(self, "36")
    }
    fn bold(&self) -> String {
        style(self, "1")
    }
    fn dimmed(&self) -> String {
        style(self, "2")
    }
    fn white(&self) -> String {
        style(self, "37")
    }
}

impl Colorize for String {
    fn red(&self) -> String {
        style(self, "31")
    }
    fn green(&self) -> String {
        style(self, "32")
    }
    fn yellow(&self) -> String {
        style(self, "33")
    }
    fn blue(&self) -> String {
        style(self, "34")
    }
    fn cyan(&self) -> String {
        style(self, "36")
    }
    fn bold(&self) -> String {
        style(self, "1")
    }
    fn dimmed(&self) -> String {
        style(self, "2")
    }
    fn white(&self) -> String {
        style(self, "37")
    }
}
