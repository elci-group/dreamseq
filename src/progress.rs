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

/// Print a one-line stage marker, always visible regardless of `--verbose`.
pub fn stage(emoji: &str, message: &str) {
    eprintln!("{emoji} {message}");
}
