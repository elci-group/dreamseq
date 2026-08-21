// Copyright (c) 2026 Dreamsequence Ltd
// SPDX-License-Identifier: MIT
//! Interactive terminal UI for editing the Dreamseq configuration file.

use crate::config::{DreamseqConfig, HarnessConfig, LogFormat};
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
    Harness(usize),
    AddHarness,
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

enum Mode {
    Normal,
    EditingText { row: Row, buffer: String },
    EditingHarness(HarnessEditor),
    ConfirmDiscard,
    ConfirmDeleteHarness(usize),
}

struct App {
    config: DreamseqConfig,
    saved_snapshot: String,
    selected: usize,
    mode: Mode,
    status: Option<String>,
    should_quit: bool,
}

impl App {
    fn new(config: DreamseqConfig) -> Result<Self> {
        let saved_snapshot = serde_json::to_string(&config)?;
        Ok(Self {
            config,
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
        ];
        for index in 0..self.config.harnesses.len() {
            rows.push(Row::Harness(index));
        }
        rows.push(Row::AddHarness);
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
                Span::styled(format!("{label:<22}"), label_style),
                Span::styled(text, style),
            ])
        };
        match row {
            Row::OutputDir => Line::from(vec![
                Span::styled(format!("{:<22}", "Output directory"), label_style),
                Span::styled(self.config.output_dir.display().to_string(), value_style),
            ]),
            Row::AnthologiesDir => Line::from(vec![
                Span::styled(format!("{:<22}", "Anthologies directory"), label_style),
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
                Span::styled(format!("{:<22}", "Groq base URL"), label_style),
                Span::styled(
                    self.config
                        .groq_base_url
                        .clone()
                        .unwrap_or_else(|| "(default routing)".to_string()),
                    value_style,
                ),
            ]),
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
            _ => {}
        }
    }

    fn save(&mut self) {
        match self.config.save() {
            Ok(()) => {
                self.saved_snapshot = serde_json::to_string(&self.config).unwrap_or_default();
                self.status = Some(format!(
                    "Saved to {}",
                    DreamseqConfig::path()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default()
                ));
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
        }
    }

    fn handle_normal_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        let rows = self.rows();
        match code {
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
            KeyCode::Char('d') => {
                if let Some(Row::Harness(index)) = rows.get(self.selected) {
                    self.mode = Mode::ConfirmDeleteHarness(*index);
                }
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                if let Some(row) = rows.get(self.selected).cloned() {
                    match row {
                        Row::OutputDir | Row::AnthologiesDir | Row::GroqBaseUrl => {
                            self.start_edit_text(row);
                        }
                        Row::EnableTts | Row::EnableKaptaind | Row::AllowRemoteAnalysis => {
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
            "↑/↓ move · Enter/Space edit · a add harness · d delete harness · s save · q quit"
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
pub fn run(config: DreamseqConfig) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(config)?;
    let result = app.run(&mut terminal);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}
