// Copyright (c) 2026 Dreamsequence Ltd
// SPDX-License-Identifier: MIT
//! Minimal terminal styling used by the human-readable renderer.
//!
//! Delegates ANSI rendering to `form3`. Colour is applied unconditionally
//! (matching this module's prior behaviour, which never gated on `NO_COLOR`
//! or TTY status), by forcing `form3`'s colour support rather than letting it
//! auto-detect.

// Each method calls `form3::compat::Colorize` by full path (UFCS) rather than
// importing it, so its trait methods never come into scope here and collide
// with this module's own `Colorize`.

fn force_style(styled: form3::compat::StyledText) -> String {
    styled
        .with_color_support(form3::term::ColorSupport::Standard16)
        .to_string()
}

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

impl Colorize for str {
    fn red(&self) -> String {
        force_style(form3::compat::Colorize::red(self))
    }
    fn green(&self) -> String {
        force_style(form3::compat::Colorize::green(self))
    }
    fn yellow(&self) -> String {
        force_style(form3::compat::Colorize::yellow(self))
    }
    fn blue(&self) -> String {
        force_style(form3::compat::Colorize::blue(self))
    }
    fn cyan(&self) -> String {
        force_style(form3::compat::Colorize::cyan(self))
    }
    fn bold(&self) -> String {
        force_style(form3::compat::Colorize::bold(self))
    }
    fn dimmed(&self) -> String {
        force_style(form3::compat::Colorize::dimmed(self))
    }
    fn white(&self) -> String {
        force_style(form3::compat::Colorize::white(self))
    }
}

impl Colorize for String {
    fn red(&self) -> String {
        self.as_str().red()
    }
    fn green(&self) -> String {
        self.as_str().green()
    }
    fn yellow(&self) -> String {
        self.as_str().yellow()
    }
    fn blue(&self) -> String {
        self.as_str().blue()
    }
    fn cyan(&self) -> String {
        self.as_str().cyan()
    }
    fn bold(&self) -> String {
        self.as_str().bold()
    }
    fn dimmed(&self) -> String {
        self.as_str().dimmed()
    }
    fn white(&self) -> String {
        self.as_str().white()
    }
}
