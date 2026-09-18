// Copyright (c) 2026 Dreamsequence Ltd
// SPDX-License-Identifier: MIT
//! Direct, always-on terminal progress reporting for `dreamseq run`,
//! independent of the `tracing` verbosity system. `--verbose` controls how
//! much diagnostic detail `tracing` emits; without it, the default filter
//! (`warn`) shows nothing at all during a long inference run, which reads
//! indistinguishably from a hang. This module exists so a normal run always
//! shows *something* moving, regardless of `--verbose`.
//!
//! Everything here writes to stderr, never stdout, so `dreamseq run --json`
//! still emits clean JSON on stdout while a human watching the terminal
//! still sees what's happening. Every line here is newline-terminated and
//! self-contained — deliberately not a self-overwriting `\r` progress bar,
//! because that only stays legible when nothing else writes to the same
//! stream. It doesn't here: `tracing`'s own writer shares stderr, and under
//! real load (rate limits, retries) it writes far more than this module
//! does. A `\r` line left open between ticks gets `tracing` output spliced
//! into the middle of it. Plain scrolling lines can't be corrupted that way.

use crate::color::Colorize;
use form3::ansi::{self, AnsiColor};
use std::sync::atomic::{AtomicUsize, Ordering};

static STAGE_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// 256-color terminal palette indices (red, orange, yellow, green, cyan, magenta, purple).
const COLORS: &[u8] = &[196, 208, 226, 46, 51, 201, 165];

fn fg(code: u8) -> String {
    ansi::fg(&AnsiColor::Indexed(code))
}

/// Spinny animation frames
const SPINNERS: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Psychedelic wave animation frames
const WAVES: &[&str] = &["∿", "≈", "≋", "≋", "≈", "∿"];

/// Animated dots for loading
const DOTS: &[&str] = &["  ", ". ", ".. ", "..."];

/// Rotation animation for deep thinking
const ROTATION: &[&str] = &["▁", "▃", "▄", "▅", "▆", "▇", "█", "▇", "▆", "▅", "▄", "▃"];

/// Get colorized output for the current stage
fn colorize_stage(text: &str, index: usize) -> String {
    let color_idx = index % COLORS.len();
    format!("{}{}{}", fg(COLORS[color_idx]), text, ansi::reset())
}

/// Print a one-line stage marker, always visible regardless of `--verbose`.
pub fn stage(emoji: &str, message: &str) {
    let stage_num = STAGE_COUNTER.fetch_add(1, Ordering::SeqCst);

    let colored_emoji = colorize_stage(emoji, stage_num);
    let sparkle_left = if stage_num > 0 { "✨ " } else { "" };
    let sparkle_right = " ✨";

    eprintln!(
        "{}{} {}{}",
        sparkle_left, colored_emoji, message, sparkle_right
    );
}

/// Print a trippy progress update with animation
pub fn progress(message: &str) {
    let stage_num = STAGE_COUNTER.load(Ordering::SeqCst);
    let tick = stage_num % 10;
    let spinner_idx = tick % SPINNERS.len();
    let wave_idx = tick % WAVES.len();
    let rotation_idx = tick % ROTATION.len();

    let spinner = SPINNERS[spinner_idx];
    let wave = WAVES[wave_idx];
    let rotation = ROTATION[rotation_idx];

    let colored = colorize_stage(
        &format!("{} {} {} {}", spinner, rotation, message, wave),
        stage_num,
    );
    eprintln!("  {}", colored);
}

/// Print a completion message with celebration
pub fn complete(message: &str) {
    let celebratory = format!("🎆 {} 🎆", message);
    eprintln!();
    eprintln!("{}", celebratory.bold().green());
    eprintln!("{}", "✨ ".repeat(20));
    eprintln!();
}

/// Print a processing indicator with colorful dots
pub fn processing(label: &str) {
    let dots_idx = STAGE_COUNTER.load(Ordering::SeqCst) % DOTS.len();
    let dots = DOTS[dots_idx];
    let stage_num = STAGE_COUNTER.load(Ordering::SeqCst);
    let colored = colorize_stage(&format!("    {} {}", label, dots), stage_num);
    eprintln!("{}", colored);
}

/// Print a milestone marker with rainbow colors
pub fn milestone(emoji: &str, text: &str) {
    let stage_num = STAGE_COUNTER.load(Ordering::SeqCst);
    let colored_emoji = colorize_stage(emoji, stage_num);
    let colored_text = colorize_stage(text, stage_num + 1);
    eprintln!("{} {}", colored_emoji, colored_text);
}

/// Print decorative separator with rainbow colors
pub fn separator() {
    let line = "━".repeat(60);
    for (i, ch) in line.chars().enumerate() {
        let color_idx = i % COLORS.len();
        eprint!("{}{}{}", fg(COLORS[color_idx]), ch, ansi::reset());
    }
    eprintln!();
}

/// Rainbow progress bar (9 stages)
pub fn bar(current: usize, total: usize) {
    if total == 0 {
        return;
    }
    let percentage = (current as f64 / total as f64) * 100.0;
    let filled = (current as f64 / total as f64 * 30.0) as usize;

    let mut bar = String::new();
    for i in 0..30 {
        let color_idx = (i + current) % COLORS.len();
        if i < filled {
            bar.push_str(&format!("{}█{}", fg(COLORS[color_idx]), ansi::reset()));
        } else {
            bar.push_str(&format!("{}░{}", fg(COLORS[color_idx]), ansi::reset()));
        }
    }

    eprintln!("  [{}] {:.0}%", bar, percentage);
}
