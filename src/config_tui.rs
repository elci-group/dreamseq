// Copyright (c) 2026 Dreamsequence Ltd
// SPDX-License-Identifier: MIT
//! Interactive terminal UI for editing the Dreamseq configuration file.

use crate::config::{DreamseqConfig, HarnessConfig, LogFormat, ProviderSettings};
use crate::groq::{GroqClient, Protocol};
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use std::io;

const LOG_FORMATS: [LogFormat; 5] = [
    LogFormat::Json,
    LogFormat::Markdown,
    LogFormat::Plain,
    LogFormat::CodexSqlite,
    LogFormat::Bound,
];

fn log_format_label(format: &LogFormat) -> &'static str {
    match format {
        LogFormat::Json => "json",
        LogFormat::Markdown => "markdown",
        LogFormat::Plain => "plain",
        LogFormat::CodexSqlite => "codex_sqlite",
        LogFormat::Bound => "bound",
    }
}

fn next_log_format(format: &LogFormat) -> LogFormat {
    let index = LOG_FORMATS
        .iter()
        .position(|candidate| log_format_label(candidate) == log_format_label(format))
        .unwrap_or(0);
    LOG_FORMATS[(index + 1) % LOG_FORMATS.len()].clone()
}

fn prev_log_format(format: &LogFormat) -> LogFormat {
    let index = LOG_FORMATS
        .iter()
        .position(|candidate| log_format_label(candidate) == log_format_label(format))
        .unwrap_or(0);
    LOG_FORMATS[(index + LOG_FORMATS.len() - 1) % LOG_FORMATS.len()].clone()
}

#[derive(Clone)]
enum Row {
    OutputDir,
    AnthologiesDir,
    EnableTts,
    EnableKaptaind,
    AllowRemoteAnalysis,
    AutoApproveRemoteAnalysis,
    GroqBaseUrl,
    CloudInferenceEnabled,
    ByokFallbackEnabled,
    Harness(usize),
    AddHarness,
    Provider(usize),
    AddProvider,
}

#[derive(PartialEq)]
enum HarnessField {
    Name,
    LogPath,
    LogFormat,
    BoundFilter,
}

impl HarnessField {
    fn label(&self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::LogPath => "Log path",
            Self::LogFormat => "Log format",
            Self::BoundFilter => "Bound filter (optional)",
        }
    }

    fn next(&self) -> Self {
        match self {
            Self::Name => Self::LogPath,
            Self::LogPath => Self::LogFormat,
            Self::LogFormat => Self::BoundFilter,
            Self::BoundFilter => Self::Name,
        }
    }
}

struct HarnessEditor {
    editing_index: Option<usize>,
    name: String,
    log_path: String,
    log_format: LogFormat,
    bound_filter: String,
    field: HarnessField,
}

impl HarnessEditor {
    fn new_blank() -> Self {
        Self {
            editing_index: None,
            name: String::new(),
            log_path: String::new(),
            log_format: LogFormat::Plain,
            bound_filter: String::new(),
            field: HarnessField::Name,
        }
    }

    fn from_existing(index: usize, harness: &HarnessConfig) -> Self {
        Self {
            editing_index: Some(index),
            name: harness.name.clone(),
            log_path: harness.log_path.display().to_string(),
            log_format: harness.log_format.clone(),
            bound_filter: harness.bound_filter.clone().unwrap_or_default(),
            field: HarnessField::Name,
        }
    }

    fn into_harness(self) -> HarnessConfig {
        let bound_filter = self.bound_filter.trim();
        HarnessConfig {
            name: self.name.trim().to_string(),
            log_path: std::path::PathBuf::from(self.log_path.trim()),
            log_format: self.log_format,
            bound_filter: if bound_filter.is_empty() {
                None
            } else {
                Some(bound_filter.to_string())
            },
        }
    }
}

/// Fields cycle Name → BaseUrl → Protocol → Model → LightModel → ApiKey and
/// back, matching `HarnessField`'s pattern of "last field's Enter saves".
#[derive(PartialEq, Clone, Copy)]
enum ProviderField {
    Name,
    BaseUrl,
    Protocol,
    Model,
    LightModel,
    ApiKey,
}

impl ProviderField {
    fn label(&self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::BaseUrl => "Base URL",
            Self::Protocol => "Protocol",
            Self::Model => "Model (heavy tasks)",
            Self::LightModel => "Light model (optional)",
            Self::ApiKey => "API key",
        }
    }

    fn next(&self) -> Self {
        match self {
            Self::Name => Self::BaseUrl,
            Self::BaseUrl => Self::Protocol,
            Self::Protocol => Self::Model,
            Self::Model => Self::LightModel,
            Self::LightModel => Self::ApiKey,
            Self::ApiKey => Self::Name,
        }
    }
}

struct ProviderEditor {
    editing_index: Option<usize>,
    name: String,
    base_url: String,
    model: String,
    light_model: String,
    api_key: String,
    protocol: Protocol,
    enabled: bool,
    field: ProviderField,
}

impl ProviderEditor {
    fn new_blank() -> Self {
        Self {
            editing_index: None,
            name: String::new(),
            base_url: String::new(),
            model: String::new(),
            light_model: String::new(),
            api_key: String::new(),
            protocol: Protocol::OpenAiCompatible,
            enabled: true,
            field: ProviderField::Name,
        }
    }

    /// Pre-filled from one of `GroqClient::known_provider_templates`. Jumps
    /// straight to the API key field since that's the only thing a
    /// well-known template doesn't already supply.
    fn from_template(name: &str, base_url: &str, model: &str, protocol: Protocol) -> Self {
        Self {
            editing_index: None,
            name: name.to_string(),
            base_url: base_url.to_string(),
            model: model.to_string(),
            light_model: String::new(),
            api_key: String::new(),
            protocol,
            enabled: true,
            field: ProviderField::ApiKey,
        }
    }

    fn from_existing(index: usize, provider: &ProviderSettings) -> Self {
        Self {
            editing_index: Some(index),
            name: provider.name.clone(),
            base_url: provider.base_url.clone(),
            model: provider.model.clone(),
            light_model: provider.light_model.clone().unwrap_or_default(),
            api_key: provider.api_key.clone(),
            protocol: provider.protocol,
            enabled: provider.enabled,
            field: ProviderField::Name,
        }
    }

    fn into_provider_settings(self) -> ProviderSettings {
        let light_model = self.light_model.trim();
        ProviderSettings {
            name: self.name.trim().to_string(),
            enabled: self.enabled,
            base_url: self.base_url.trim().trim_end_matches('/').to_string(),
            model: self.model.trim().to_string(),
            light_model: if light_model.is_empty() {
                None
            } else {
                Some(light_model.to_string())
            },
            api_key: self.api_key.trim().to_string(),
            protocol: self.protocol,
        }
    }
}

enum Mode {
    Normal,
    EditingText { row: Row, buffer: String },
    EditingHarness(HarnessEditor),
    EditingProvider(ProviderEditor),
    PickingProviderTemplate { selected: usize },
    ConfirmDiscard,
    ConfirmDeleteHarness(usize),
    ConfirmDeleteProvider(usize),
}

struct App {
    config: DreamseqConfig,
    /// Where `save` writes to — the path this config was loaded from (or the
    /// default location), never silently swapped for the other. Without
    /// this, editing a config loaded via `--config <path>` would still save
    /// to the default `~/.config/dreamseq/config.json`.
    save_path: std::path::PathBuf,
    saved_snapshot: String,
    selected: usize,
    mode: Mode,
    status: Option<String>,
    should_quit: bool,
}

impl App {
    fn new(config: DreamseqConfig, save_path: std::path::PathBuf) -> Result<Self> {
        let saved_snapshot = serde_json::to_string(&config)?;
        Ok(Self {
            config,
            save_path,
            saved_snapshot,
            selected: 0,
            mode: Mode::Normal,
            status: None,
            should_quit: false,
        })
    }

    fn is_dirty(&self) -> bool {
        serde_json::to_string(&self.config).unwrap_or_default() != self.saved_snapshot
    }

    fn rows(&self) -> Vec<Row> {
        let mut rows = vec![
            Row::OutputDir,
            Row::AnthologiesDir,
            Row::EnableTts,
            Row::EnableKaptaind,
            Row::AllowRemoteAnalysis,
            Row::AutoApproveRemoteAnalysis,
            Row::GroqBaseUrl,
            Row::CloudInferenceEnabled,
            Row::ByokFallbackEnabled,
        ];
        for index in 0..self.config.harnesses.len() {
            rows.push(Row::Harness(index));
        }
        rows.push(Row::AddHarness);
        for index in 0..self.config.providers.len() {
            rows.push(Row::Provider(index));
        }
        rows.push(Row::AddProvider);
        rows
    }

    fn row_line(&self, row: &Row) -> Line<'static> {
        let label_style = Style::default().fg(Color::Gray);
        let value_style = Style::default().fg(Color::Cyan);
        let bool_line = |label: &'static str, value: bool| {
            let (text, style) = if value {
                ("on", Style::default().fg(Color::Green))
            } else {
                ("off", Style::default().fg(Color::Red))
            };
            Line::from(vec![
                // The trailing space guarantees a gap even when a label (like
                // "Auto-approve remote analysis") is exactly as wide as the
                // padding — `{:<28}` alone would then butt straight into the
                // value with no separator.
                Span::styled(format!("{label:<28} "), label_style),
                Span::styled(text, style),
            ])
        };
        match row {
            Row::OutputDir => Line::from(vec![
                Span::styled(format!("{:<28}", "Output directory"), label_style),
                Span::styled(self.config.output_dir.display().to_string(), value_style),
            ]),
            Row::AnthologiesDir => Line::from(vec![
                Span::styled(format!("{:<28}", "Anthologies directory"), label_style),
                Span::styled(
                    self.config.anthologies_dir.display().to_string(),
                    value_style,
                ),
            ]),
            Row::EnableTts => bool_line("Voice notifications", self.config.enable_tts),
            Row::EnableKaptaind => bool_line("Kaptaind monitor", self.config.enable_kaptaind),
            Row::AllowRemoteAnalysis => {
                bool_line("Allow remote analysis", self.config.allow_remote_analysis)
            }
            Row::AutoApproveRemoteAnalysis => bool_line(
                "Auto-approve remote analysis",
                self.config.auto_approve_remote_analysis,
            ),
            Row::GroqBaseUrl => Line::from(vec![
                Span::styled(format!("{:<28}", "Groq base URL"), label_style),
                Span::styled(
                    self.config
                        .groq_base_url
                        .clone()
                        .unwrap_or_else(|| "(default routing)".to_string()),
                    value_style,
                ),
            ]),
            Row::CloudInferenceEnabled => bool_line(
                "Dreamsequence cloud route",
                self.config.cloud_inference_enabled,
            ),
            Row::ByokFallbackEnabled => {
                bool_line("BYOK fallback", self.config.byok_fallback_enabled)
            }
            Row::Harness(index) => {
                let harness = &self.config.harnesses[*index];
                Line::from(vec![
                    Span::styled("  • ", Style::default().fg(Color::DarkGray)),
                    Span::styled(harness.name.clone(), Style::default().fg(Color::Yellow)),
                    Span::raw("  "),
                    Span::styled(
                        format!("[{}]", log_format_label(&harness.log_format)),
                        Style::default().fg(Color::Magenta),
                    ),
                    Span::raw("  "),
                    Span::styled(
                        harness.log_path.display().to_string(),
                        Style::default().fg(Color::DarkGray),
                    ),
                ])
            }
            Row::AddHarness => Line::from(Span::styled(
                "  + Add harness",
                Style::default().fg(Color::Green),
            )),
            Row::Provider(index) => {
                let provider = &self.config.providers[*index];
                let (marker, marker_style) = if provider.enabled {
                    ("●", Style::default().fg(Color::Green))
                } else {
                    ("○", Style::default().fg(Color::DarkGray))
                };
                let mut spans = vec![
                    Span::styled(format!("  {}. ", index + 1), Style::default().fg(Color::DarkGray)),
                    Span::styled(marker, marker_style),
                    Span::raw(" "),
                    Span::styled(provider.name.clone(), Style::default().fg(Color::Yellow)),
                    Span::raw("  "),
                    Span::styled(provider.model.clone(), Style::default().fg(Color::Cyan)),
                ];
                if let Some(light) = &provider.light_model {
                    spans.push(Span::raw(" / "));
                    spans.push(Span::styled(
                        light.clone(),
                        Style::default().fg(Color::DarkGray),
                    ));
                }
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    provider.base_url.clone(),
                    Style::default().fg(Color::DarkGray),
                ));
                if provider.api_key.trim().is_empty() {
                    spans.push(Span::raw("  "));
                    spans.push(Span::styled(
                        "(no key stored)",
                        Style::default().fg(Color::Red),
                    ));
                }
                Line::from(spans)
            }
            Row::AddProvider => Line::from(Span::styled(
                "  + Add provider",
                Style::default().fg(Color::Green),
            )),
        }
    }

    fn start_edit_text(&mut self, row: Row) {
        let buffer = match &row {
            Row::OutputDir => self.config.output_dir.display().to_string(),
            Row::AnthologiesDir => self.config.anthologies_dir.display().to_string(),
            Row::GroqBaseUrl => self.config.groq_base_url.clone().unwrap_or_default(),
            _ => String::new(),
        };
        self.mode = Mode::EditingText { row, buffer };
    }

    fn commit_text(&mut self, row: Row, buffer: String) {
        let trimmed = buffer.trim().to_string();
        match row {
            Row::OutputDir if !trimmed.is_empty() => {
                self.config.output_dir = std::path::PathBuf::from(trimmed);
            }
            Row::AnthologiesDir if !trimmed.is_empty() => {
                self.config.anthologies_dir = std::path::PathBuf::from(trimmed);
            }
            Row::GroqBaseUrl => {
                self.config.groq_base_url = if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed)
                };
            }
            _ => {}
        }
        self.mode = Mode::Normal;
    }

    fn toggle_bool(&mut self, row: &Row) {
        match row {
            Row::EnableTts => self.config.enable_tts = !self.config.enable_tts,
            Row::EnableKaptaind => self.config.enable_kaptaind = !self.config.enable_kaptaind,
            Row::AllowRemoteAnalysis => {
                self.config.allow_remote_analysis = !self.config.allow_remote_analysis
            }
            Row::AutoApproveRemoteAnalysis => {
                self.config.auto_approve_remote_analysis =
                    !self.config.auto_approve_remote_analysis
            }
            Row::CloudInferenceEnabled => {
                self.config.cloud_inference_enabled = !self.config.cloud_inference_enabled
            }
            Row::ByokFallbackEnabled => {
                self.config.byok_fallback_enabled = !self.config.byok_fallback_enabled
            }
            _ => {}
        }
    }

    /// Swaps the currently selected provider with its neighbor `delta`
    /// positions away — this list's order *is* the fallback hierarchy, so
    /// reordering here is how a user re-prioritizes providers.
    fn move_selected_provider(&mut self, delta: i32) {
        let rows = self.rows();
        let Some(Row::Provider(index)) = rows.get(self.selected).cloned() else {
            return;
        };
        let target = index as i32 + delta;
        if target < 0 || target as usize >= self.config.providers.len() {
            return;
        }
        self.config.providers.swap(index, target as usize);
        self.selected = (self.selected as i32 + delta).max(0) as usize;
    }

    fn save(&mut self) {
        match self.config.save_to_path(&self.save_path) {
            Ok(()) => {
                self.saved_snapshot = serde_json::to_string(&self.config).unwrap_or_default();
                self.status = Some(format!("Saved to {}", self.save_path.display()));
            }
            Err(error) => {
                tracing::warn!(error = %error, "failed to save dreamseq configuration");
                self.status = Some(format!("Save failed: {error}"));
            }
        }
    }

    fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        self.status = None;
        match &mut self.mode {
            Mode::Normal => self.handle_normal_key(code, modifiers),
            Mode::EditingText { .. } => self.handle_text_key(code),
            Mode::EditingHarness(_) => self.handle_harness_key(code),
            Mode::EditingProvider(_) => self.handle_provider_key(code),
            Mode::PickingProviderTemplate { .. } => self.handle_template_picker_key(code),
            Mode::ConfirmDiscard => match code {
                KeyCode::Char('s') | KeyCode::Char('S') => {
                    self.save();
                    self.should_quit = true;
                }
                KeyCode::Char('d') | KeyCode::Char('D') => self.should_quit = true,
                KeyCode::Esc => self.mode = Mode::Normal,
                _ => {}
            },
            Mode::ConfirmDeleteHarness(index) => match code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    let index = *index;
                    self.config.harnesses.remove(index);
                    let row_count = self.rows().len();
                    if self.selected >= row_count {
                        self.selected = row_count.saturating_sub(1);
                    }
                    self.mode = Mode::Normal;
                }
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                    self.mode = Mode::Normal;
                }
                _ => {}
            },
            Mode::ConfirmDeleteProvider(index) => match code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    let index = *index;
                    self.config.providers.remove(index);
                    let row_count = self.rows().len();
                    if self.selected >= row_count {
                        self.selected = row_count.saturating_sub(1);
                    }
                    self.mode = Mode::Normal;
                }
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                    self.mode = Mode::Normal;
                }
                _ => {}
            },
        }
    }

    fn handle_normal_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        let rows = self.rows();
        match code {
            KeyCode::Up if modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_selected_provider(-1);
            }
            KeyCode::Down if modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_selected_provider(1);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.selected + 1 < rows.len() {
                    self.selected += 1;
                }
            }
            KeyCode::Char('s') if modifiers.contains(KeyModifiers::CONTROL) => self.save(),
            KeyCode::Char('s') => self.save(),
            KeyCode::Char('q') | KeyCode::Esc => {
                if self.is_dirty() {
                    self.mode = Mode::ConfirmDiscard;
                } else {
                    self.should_quit = true;
                }
            }
            KeyCode::Char('a') => {
                self.mode = Mode::EditingHarness(HarnessEditor::new_blank());
            }
            KeyCode::Char('p') => {
                self.mode = Mode::PickingProviderTemplate { selected: 0 };
            }
            KeyCode::Char('d') => match rows.get(self.selected) {
                Some(Row::Harness(index)) => self.mode = Mode::ConfirmDeleteHarness(*index),
                Some(Row::Provider(index)) => self.mode = Mode::ConfirmDeleteProvider(*index),
                _ => {}
            },
            KeyCode::Enter | KeyCode::Char(' ') => {
                if let Some(row) = rows.get(self.selected).cloned() {
                    match row {
                        Row::OutputDir | Row::AnthologiesDir | Row::GroqBaseUrl => {
                            self.start_edit_text(row);
                        }
                        Row::EnableTts
                        | Row::EnableKaptaind
                        | Row::AllowRemoteAnalysis
                        | Row::AutoApproveRemoteAnalysis
                        | Row::CloudInferenceEnabled
                        | Row::ByokFallbackEnabled => {
                            self.toggle_bool(&row);
                        }
                        Row::Harness(index) => {
                            let harness = self.config.harnesses[index].clone();
                            self.mode =
                                Mode::EditingHarness(HarnessEditor::from_existing(index, &harness));
                        }
                        Row::AddHarness => {
                            self.mode = Mode::EditingHarness(HarnessEditor::new_blank());
                        }
                        // Space quickly flips a provider on/off from the list;
                        // Enter (falling to the arm below) opens the full editor.
                        Row::Provider(index) if code == KeyCode::Char(' ') => {
                            self.config.providers[index].enabled =
                                !self.config.providers[index].enabled;
                        }
                        Row::Provider(index) => {
                            let provider = self.config.providers[index].clone();
                            self.mode = Mode::EditingProvider(ProviderEditor::from_existing(
                                index, &provider,
                            ));
                        }
                        Row::AddProvider => {
                            self.mode = Mode::PickingProviderTemplate { selected: 0 };
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn handle_text_key(&mut self, code: KeyCode) {
        let Mode::EditingText { row, buffer } = &mut self.mode else {
            return;
        };
        match code {
            KeyCode::Enter => {
                let row = row.clone();
                let buffer = std::mem::take(buffer);
                self.commit_text(row, buffer);
            }
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Backspace => {
                buffer.pop();
            }
            KeyCode::Char(character) => buffer.push(character),
            _ => {}
        }
    }

    fn handle_harness_key(&mut self, code: KeyCode) {
        let Mode::EditingHarness(editor) = &mut self.mode else {
            return;
        };
        match code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Tab | KeyCode::Down => editor.field = editor.field.next(),
            KeyCode::BackTab | KeyCode::Up => {
                for _ in 0..3 {
                    editor.field = editor.field.next();
                }
            }
            KeyCode::Left if editor.field == HarnessField::LogFormat => {
                editor.log_format = prev_log_format(&editor.log_format);
            }
            KeyCode::Right if editor.field == HarnessField::LogFormat => {
                editor.log_format = next_log_format(&editor.log_format);
            }
            KeyCode::Enter if editor.field == HarnessField::LogFormat => {
                editor.log_format = next_log_format(&editor.log_format);
            }
            KeyCode::Enter => {
                if editor.field == HarnessField::BoundFilter {
                    if let Mode::EditingHarness(editor) =
                        std::mem::replace(&mut self.mode, Mode::Normal)
                    {
                        if editor.name.trim().is_empty() || editor.log_path.trim().is_empty() {
                            self.status = Some("Harness needs a name and a log path.".to_string());
                            self.mode = Mode::EditingHarness(editor);
                            return;
                        }
                        let editing_index = editor.editing_index;
                        let harness = editor.into_harness();
                        match editing_index {
                            Some(index) => self.config.harnesses[index] = harness,
                            None => self.config.harnesses.push(harness),
                        }
                    }
                } else {
                    editor.field = editor.field.next();
                }
            }
            KeyCode::Backspace => match editor.field {
                HarnessField::Name => {
                    editor.name.pop();
                }
                HarnessField::LogPath => {
                    editor.log_path.pop();
                }
                HarnessField::LogFormat => {}
                HarnessField::BoundFilter => {
                    editor.bound_filter.pop();
                }
            },
            KeyCode::Char(character) => match editor.field {
                HarnessField::Name => editor.name.push(character),
                HarnessField::LogPath => editor.log_path.push(character),
                HarnessField::LogFormat => {}
                HarnessField::BoundFilter => editor.bound_filter.push(character),
            },
            _ => {}
        }
    }

    fn handle_provider_key(&mut self, code: KeyCode) {
        let Mode::EditingProvider(editor) = &mut self.mode else {
            return;
        };
        match code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Tab | KeyCode::Down => editor.field = editor.field.next(),
            KeyCode::BackTab | KeyCode::Up => {
                for _ in 0..5 {
                    editor.field = editor.field.next();
                }
            }
            KeyCode::Left if editor.field == ProviderField::Protocol => {
                editor.protocol = editor.protocol.toggled();
            }
            KeyCode::Right if editor.field == ProviderField::Protocol => {
                editor.protocol = editor.protocol.toggled();
            }
            KeyCode::Enter if editor.field == ProviderField::Protocol => {
                editor.protocol = editor.protocol.toggled();
            }
            KeyCode::Enter => {
                if editor.field == ProviderField::ApiKey {
                    if let Mode::EditingProvider(editor) =
                        std::mem::replace(&mut self.mode, Mode::Normal)
                    {
                        if editor.name.trim().is_empty()
                            || editor.base_url.trim().is_empty()
                            || editor.model.trim().is_empty()
                        {
                            self.status =
                                Some("Provider needs a name, base URL, and model.".to_string());
                            self.mode = Mode::EditingProvider(editor);
                            return;
                        }
                        let editing_index = editor.editing_index;
                        let provider = editor.into_provider_settings();
                        match editing_index {
                            Some(index) => self.config.providers[index] = provider,
                            None => self.config.providers.push(provider),
                        }
                    }
                } else {
                    editor.field = editor.field.next();
                }
            }
            KeyCode::Backspace => match editor.field {
                ProviderField::Name => {
                    editor.name.pop();
                }
                ProviderField::BaseUrl => {
                    editor.base_url.pop();
                }
                ProviderField::Protocol => {}
                ProviderField::Model => {
                    editor.model.pop();
                }
                ProviderField::LightModel => {
                    editor.light_model.pop();
                }
                ProviderField::ApiKey => {
                    editor.api_key.pop();
                }
            },
            KeyCode::Char(character) => match editor.field {
                ProviderField::Name => editor.name.push(character),
                ProviderField::BaseUrl => editor.base_url.push(character),
                ProviderField::Protocol => {}
                ProviderField::Model => editor.model.push(character),
                ProviderField::LightModel => editor.light_model.push(character),
                ProviderField::ApiKey => editor.api_key.push(character),
            },
            _ => {}
        }
    }

    fn handle_template_picker_key(&mut self, code: KeyCode) {
        let Mode::PickingProviderTemplate { selected } = &mut self.mode else {
            return;
        };
        let templates = GroqClient::known_provider_templates();
        let total = templates.len() + 1; // +1 for the trailing "custom" option
        match code {
            KeyCode::Up | KeyCode::Char('k') => *selected = selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                if *selected + 1 < total {
                    *selected += 1;
                }
            }
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Enter => {
                let selected = *selected;
                let editor = if selected < templates.len() {
                    let (name, base_url, model, protocol) = templates[selected];
                    ProviderEditor::from_template(name, base_url, model, protocol)
                } else {
                    ProviderEditor::new_blank()
                };
                self.mode = Mode::EditingProvider(editor);
            }
            _ => {}
        }
    }

    fn draw(&mut self, frame: &mut ratatui::Frame) {
        let area = frame.area();
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(5),
                Constraint::Length(3),
            ])
            .split(area);

        let title = Paragraph::new(Line::from(vec![
            Span::styled(
                "🌙 dreamseq config",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                if self.is_dirty() {
                    "● unsaved"
                } else {
                    "✓ saved"
                },
                Style::default().fg(if self.is_dirty() {
                    Color::Yellow
                } else {
                    Color::Green
                }),
            ),
        ]))
        .block(Block::default().borders(Borders::ALL))
        .alignment(Alignment::Left);
        frame.render_widget(title, layout[0]);

        let rows = self.rows();
        let items: Vec<ListItem> = rows
            .iter()
            .map(|row| ListItem::new(self.row_line(row)))
            .collect();
        let mut state = ListState::default();
        state.select(Some(self.selected));
        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(" Settings "))
            .highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("➤ ");
        frame.render_stateful_widget(list, layout[1], &mut state);

        let footer_text = self.status.clone().unwrap_or_else(|| {
            "↑/↓ move · Enter/Space edit · a harness · p provider · Ctrl+↑/↓ reorder provider · d delete · s save · q quit"
                .to_string()
        });
        let footer = Paragraph::new(footer_text)
            .style(Style::default().fg(Color::DarkGray))
            .block(Block::default().borders(Borders::ALL));
        frame.render_widget(footer, layout[2]);

        match &self.mode {
            Mode::EditingText { row, buffer } => {
                let label = match row {
                    Row::OutputDir => "Output directory",
                    Row::AnthologiesDir => "Anthologies directory",
                    Row::GroqBaseUrl => "Groq base URL (blank for default routing)",
                    _ => "Value",
                };
                draw_text_popup(frame, area, label, buffer);
            }
            Mode::EditingHarness(editor) => draw_harness_popup(frame, area, editor),
            Mode::EditingProvider(editor) => draw_provider_popup(frame, area, editor),
            Mode::PickingProviderTemplate { selected } => {
                draw_template_picker(frame, area, *selected)
            }
            Mode::ConfirmDiscard => draw_confirm_popup(
                frame,
                area,
                "Unsaved changes",
                "You have unsaved changes. [s] save & quit  [d] discard & quit  [Esc] cancel",
            ),
            Mode::ConfirmDeleteHarness(index) => {
                let name = self
                    .config
                    .harnesses
                    .get(*index)
                    .map(|harness| harness.name.as_str())
                    .unwrap_or("this harness");
                draw_confirm_popup(
                    frame,
                    area,
                    "Delete harness",
                    &format!("Remove '{name}'? [y] yes  [n] cancel"),
                );
            }
            Mode::ConfirmDeleteProvider(index) => {
                let name = self
                    .config
                    .providers
                    .get(*index)
                    .map(|provider| provider.name.as_str())
                    .unwrap_or("this provider");
                draw_confirm_popup(
                    frame,
                    area,
                    "Delete provider",
                    &format!("Remove '{name}'? [y] yes  [n] cancel"),
                );
            }
            Mode::Normal => {}
        }
    }

    fn run<B: Backend>(&mut self, terminal: &mut Terminal<B>) -> Result<()> {
        while !self.should_quit {
            terminal
                .draw(|frame| self.draw(frame))
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            if let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                self.handle_key(key.code, key.modifiers);
            }
        }
        Ok(())
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

fn draw_text_popup(frame: &mut ratatui::Frame, area: Rect, label: &str, buffer: &str) {
    let popup = centered_rect(60, 20, area);
    frame.render_widget(Clear, popup);
    let text = Paragraph::new(vec![
        Line::from(Span::styled(label, Style::default().fg(Color::Gray))),
        Line::from(Span::styled(
            format!("{buffer}▏"),
            Style::default().fg(Color::White),
        )),
        Line::from(Span::styled(
            "Enter confirm · Esc cancel",
            Style::default().fg(Color::DarkGray),
        )),
    ])
    .wrap(Wrap { trim: false })
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Edit ")
            .border_style(Style::default().fg(Color::Cyan)),
    );
    frame.render_widget(text, popup);
}

fn draw_harness_popup(frame: &mut ratatui::Frame, area: Rect, editor: &HarnessEditor) {
    let popup = centered_rect(70, 45, area);
    frame.render_widget(Clear, popup);
    let fields = [
        HarnessField::Name,
        HarnessField::LogPath,
        HarnessField::LogFormat,
        HarnessField::BoundFilter,
    ];
    let mut lines = Vec::new();
    let title = if editor.editing_index.is_some() {
        " Edit harness "
    } else {
        " Add harness "
    };
    for field in fields {
        let active = field == editor.field;
        let value = match field {
            HarnessField::Name => editor.name.clone(),
            HarnessField::LogPath => editor.log_path.clone(),
            HarnessField::LogFormat => log_format_label(&editor.log_format).to_string(),
            HarnessField::BoundFilter => editor.bound_filter.clone(),
        };
        let marker = if active { "➤ " } else { "  " };
        let label_style = if active {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        let cursor = if active { "▏" } else { "" };
        lines.push(Line::from(vec![
            Span::raw(marker),
            Span::styled(format!("{:<24}", field.label()), label_style),
            Span::styled(
                format!("{value}{cursor}"),
                Style::default().fg(Color::White),
            ),
        ]));
        lines.push(Line::raw(""));
    }
    lines.push(Line::from(Span::styled(
        "Tab next field · ←/→ cycle format · Enter next/save · Esc cancel",
        Style::default().fg(Color::DarkGray),
    )));
    let widget = Paragraph::new(lines).wrap(Wrap { trim: false }).block(
        Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(Color::Cyan)),
    );
    frame.render_widget(widget, popup);
}

fn draw_provider_popup(frame: &mut ratatui::Frame, area: Rect, editor: &ProviderEditor) {
    let popup = centered_rect(72, 55, area);
    frame.render_widget(Clear, popup);
    let fields = [
        ProviderField::Name,
        ProviderField::BaseUrl,
        ProviderField::Protocol,
        ProviderField::Model,
        ProviderField::LightModel,
        ProviderField::ApiKey,
    ];
    let mut lines = Vec::new();
    let title = if editor.editing_index.is_some() {
        " Edit provider "
    } else {
        " Add provider "
    };
    for field in fields {
        let active = field == editor.field;
        let value = match field {
            ProviderField::Name => editor.name.clone(),
            ProviderField::BaseUrl => editor.base_url.clone(),
            ProviderField::Protocol => editor.protocol.label().to_string(),
            ProviderField::Model => editor.model.clone(),
            ProviderField::LightModel => editor.light_model.clone(),
            // Masked so the key doesn't linger on screen (or in a
            // screen-share) just because someone opened the editor to
            // change something else about the provider.
            ProviderField::ApiKey => "•".repeat(editor.api_key.chars().count()),
        };
        let marker = if active { "➤ " } else { "  " };
        let label_style = if active {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        let cursor = if active { "▏" } else { "" };
        lines.push(Line::from(vec![
            Span::raw(marker),
            Span::styled(format!("{:<24}", field.label()), label_style),
            Span::styled(
                format!("{value}{cursor}"),
                Style::default().fg(Color::White),
            ),
        ]));
        lines.push(Line::raw(""));
    }
    lines.push(Line::from(Span::styled(
        "Tab next field · ←/→ cycle protocol · Enter next/save · Esc cancel",
        Style::default().fg(Color::DarkGray),
    )));
    let widget = Paragraph::new(lines).wrap(Wrap { trim: false }).block(
        Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(Color::Cyan)),
    );
    frame.render_widget(widget, popup);
}

fn draw_template_picker(frame: &mut ratatui::Frame, area: Rect, selected: usize) {
    let popup = centered_rect(64, 55, area);
    frame.render_widget(Clear, popup);
    let templates = GroqClient::known_provider_templates();
    let mut lines = Vec::new();
    for (index, (name, base_url, model, _protocol)) in templates.iter().enumerate() {
        let active = index == selected;
        let marker = if active { "➤ " } else { "  " };
        let style = if active {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        lines.push(Line::from(vec![
            Span::raw(marker),
            Span::styled(format!("{name:<12}"), style),
            Span::styled(
                format!("{model}  ({base_url})"),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }
    let custom_index = templates.len();
    let active = custom_index == selected;
    let marker = if active { "➤ " } else { "  " };
    let style = if active {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };
    lines.push(Line::from(vec![
        Span::raw(marker),
        Span::styled(format!("{:<12}", "custom"), style),
        Span::styled(
            "enter every field manually",
            Style::default().fg(Color::DarkGray),
        ),
    ]));
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "↑/↓ choose · Enter select · Esc cancel",
        Style::default().fg(Color::DarkGray),
    )));
    let widget = Paragraph::new(lines).wrap(Wrap { trim: false }).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Add provider ")
            .border_style(Style::default().fg(Color::Cyan)),
    );
    frame.render_widget(widget, popup);
}

fn draw_confirm_popup(frame: &mut ratatui::Frame, area: Rect, title: &str, message: &str) {
    let popup = centered_rect(60, 20, area);
    frame.render_widget(Clear, popup);
    let widget = Paragraph::new(message).wrap(Wrap { trim: true }).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" {title} "))
            .border_style(Style::default().fg(Color::Yellow)),
    );
    frame.render_widget(widget, popup);
}

/// Launch the interactive configuration editor. Returns once the user quits.
///
/// `save_path` is the file `save` writes to — pass the same path the config
/// was loaded from (or `None` for the default location) so a session
/// started with `--config <path>` doesn't silently save somewhere else.
pub fn run(config: DreamseqConfig, save_path: Option<std::path::PathBuf>) -> Result<()> {
    let save_path = match save_path {
        Some(path) => path,
        None => DreamseqConfig::path()?,
    };

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(config, save_path)?;
    let result = app.run(&mut terminal);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}
