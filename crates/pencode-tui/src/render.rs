//! Rendering: opencode-style layout with brand header, transcript pane,
//! right-side sessions sidebar, slash-command popup, rounded prompt input,
//! status bar and help overlay.

use super::theme;
use pencode_protocol::{Message, Part, Role};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::symbols::border;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, Paragraph};
use ratatui::Frame;

pub const BRAND: &str = "pencode";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarState {
    Closed,
    Open { selected: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpState {
    Hidden,
    Visible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamStatus {
    Idle,
    Thinking,
    Streaming,
}

/// Slash commands available in the prompt; kept in render for the popup.
pub const COMMANDS: &[(&str, &str)] = &[
    ("/help", "show all keybinds"),
    ("/new", "start a new session"),
    ("/clear", "alias for /new"),
    ("/sessions", "toggle the sessions sidebar"),
    ("/model <provider/model>", "switch model, e.g. openai/gpt-4.1"),
    ("/themes", "toggle dark/light theme"),
    ("/agent", "toggle build/plan agent mode"),
    ("/compact", "summarize this session to free context"),
    ("/undo", "remove the last exchange"),
    ("/retry", "re-send the last prompt"),
    ("/init", "generate an AGENTS.md for this repo"),
    ("/version", "print version info"),
    ("/exit", "quit pencode"),
];

pub struct UiState {
    pub input: String,
    /// Lines scrolled back from the newest message (0 = pinned to bottom).
    pub scroll_back: usize,
    pub sidebar: SidebarState,
    pub help: HelpState,
    pub sessions: Vec<pencode_protocol::Session>,
    pub status: StreamStatus,
    pub error: Option<String>,
    pub notice: Option<String>,
    /// Session id a completion is currently streaming into.
    pub streaming_message: Option<String>,
    /// Filtered slash-command matches while input starts with `/`.
    pub command_matches: Vec<(&'static str, &'static str)>,
    pub command_selected: usize,
    /// `true` while `/model` set a plan-mode style restriction.
    pub plan_mode: bool,
    /// Active model spec shown in the footer.
    pub model_spec: String,
}

impl UiState {
    pub fn refresh_sessions(&mut self, store: &pencode_core::session::Store) -> anyhow::Result<()> {
        self.sessions = store.list()?;
        Ok(())
    }

    pub fn update_command_matches(&mut self) {
        if !self.input.starts_with('/') || self.input.contains(' ') {
            self.command_matches.clear();
            self.command_selected = 0;
            return;
        }
        let query = self.input.to_lowercase();
        self.command_matches = COMMANDS
            .iter()
            .filter(|(name, _)| name.split(' ').next().unwrap_or(name).starts_with(&query))
            .copied()
            .collect();
        self.command_selected = 0;
    }

    /// Replace the input with the highlighted command (keeps trailing space
    /// for commands that take arguments).
    pub fn accept_command(&mut self) {
        if let Some((name, _)) =
            self.command_matches.get(self.command_selected).copied()
        {
            let takes_args = name.contains(' ');
            self.input = if takes_args {
                format!("{name} ")
            } else {
                name.to_string()
            };
            self.update_command_matches();
        }
    }
}

pub fn draw(
    frame: &mut Frame,
    session: &pencode_protocol::Session,
    state: &UiState,
    directory: &str,
) {
    let area = frame.area();
    let (main_area, sidebar_area) = match state.sidebar {
        SidebarState::Closed => (area, None),
        SidebarState::Open { .. } => {
            let [main, side] =
                Layout::horizontal([Constraint::Min(30), Constraint::Length(34)]).areas(area);
            (main, Some(side))
        }
    };

    draw_main(frame, main_area, session, state, directory);

    if let (Some(side), SidebarState::Open { selected }) = (sidebar_area, state.sidebar) {
        draw_sidebar(frame, side, state, selected, directory);
    }

    if !state.command_matches.is_empty() {
        draw_command_popup(frame, state, area);
    }

    if matches!(state.help, HelpState::Visible) {
        draw_help(frame, area);
    }
}

fn draw_main(
    frame: &mut Frame,
    area: Rect,
    session: &pencode_protocol::Session,
    state: &UiState,
    directory: &str,
) {
    let [header_area, transcript_area, input_area, footer_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
    .areas(area);

    // Header: brand + agent mode + session title + dir + status/notice.
    let t = theme::active();
    let dir_label = shorten_path(directory, 28);
    let status_text = match state.status {
        StreamStatus::Idle => match (&state.error, &state.notice) {
            (Some(err), _) => format!("  ⚠ {}", truncate(err, 48)),
            (None, Some(notice)) => format!("  ✓ {}", truncate(notice, 48)),
            (None, None) => String::new(),
        },
        StreamStatus::Thinking => "  ◍ thinking…".to_string(),
        StreamStatus::Streaming => "  ✻ streaming…".to_string(),
    };
    let status_color = match (&state.status, &state.error, &state.notice) {
        (StreamStatus::Idle, Some(_), _) => t.error,
        (StreamStatus::Idle, None, Some(_)) => t.success,
        (StreamStatus::Idle, None, None) => t.weak,
        (StreamStatus::Thinking, _, _) => t.warning,
        (StreamStatus::Streaming, _, _) => t.accent,
    };
    let mode_span = if state.plan_mode {
        Span::styled(" plan ", theme::bold(t.warning))
    } else {
        Span::styled(" build ", theme::bold(t.success))
    };
    let fixed = dir_label.chars().count() as u16 + status_text.chars().count() as u16 + 34;
    let title_width = area.width.saturating_sub(fixed).max(8) as usize;
    let header = Line::from(vec![
        Span::styled(" ◆ ", theme::bold(t.primary)),
        Span::styled(BRAND, theme::bold(t.primary)),
        Span::styled("  │ ", theme::fg(t.weak)),
        mode_span,
        Span::styled(" │  ", theme::fg(t.weak)),
        Span::styled(truncate(&session.title, title_width), theme::fg(t.ink)),
        Span::styled(format!("  ({dir_label})"), theme::fg(t.weak)),
        Span::styled(status_text, theme::fg(status_color)),
    ]);
    frame.render_widget(Paragraph::new(header), header_area);

    // Transcript: role-labelled message blocks pinned to the bottom.
    let inner_width = transcript_area.width.max(20) as usize;
    let inner_height = transcript_area.height.saturating_sub(2).max(1) as usize;

    let mut lines: Vec<Line> = Vec::new();
    for message in &session.messages {
        lines.extend(message_lines(message, inner_width));
    }
    if lines.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  type a prompt and press enter — try / for commands",
            theme::bold(t.primary),
        )));
        lines.push(Line::from(Span::styled(
            "  tab sessions · ? help · messages persist locally",
            theme::fg(t.weak),
        )));
    }

    let total = lines.len();
    let max_scroll = total.saturating_sub(inner_height);
    let offset = max_scroll.saturating_sub(state.scroll_back.min(max_scroll));
    let transcript_block = Block::bordered()
        .border_set(border::ROUNDED)
        .border_style(theme::fg(t.weak));
    frame.render_widget(transcript_block, transcript_area);
    frame.render_widget(
        Paragraph::new(lines)
            .style(Style::default().bg(t.bg))
            .scroll((offset as u16, 0)),
        transcript_area,
    );

    // Prompt input with rounded border and visible cursor.
    let input_block = Block::bordered()
        .border_set(border::ROUNDED)
        .border_style(if state.input.starts_with('/') {
            theme::fg(t.accent)
        } else if state.input.is_empty() {
            theme::fg(t.weak)
        } else {
            theme::fg(t.primary)
        })
        .title(Span::styled(" prompt ", theme::fg(t.primary)));
    frame.render_widget(input_block, input_area);
    frame.render_widget(
        Paragraph::new(state.input.as_str()).style(theme::fg(t.ink)),
        ratatui::widgets::Block::bordered()
            .border_set(border::ROUNDED)
            .inner(input_area),
    );
    let cursor_x = input_area.x
        + 1
        + state
            .input
            .chars()
            .count()
            .min(input_area.width.saturating_sub(3) as usize) as u16;
    frame.set_cursor_position((cursor_x, input_area.y + 1));

    // Footer status bar.
    let short_session: String = session.id.chars().take(13).collect();
    let footer = Line::from(vec![
        Span::styled(format!(" {} ", state.model_spec), theme::fg(t.info)),
        Span::styled("│", theme::fg(t.weak)),
        Span::styled(format!(" v{VERSION} "), theme::fg(t.weak)),
        Span::styled("│", theme::fg(t.weak)),
        Span::styled(format!(" {short_session} "), theme::fg(t.weak)),
        Span::styled("│", theme::fg(t.weak)),
        Span::styled(" tab sessions ", theme::fg(t.accent)),
        Span::styled("·", theme::fg(t.weak)),
        Span::styled(" ? help ", theme::fg(t.accent)),
        Span::styled("·", theme::fg(t.weak)),
        Span::styled(" enter send ", theme::fg(t.success)),
        Span::styled("·", theme::fg(t.weak)),
        Span::styled(" esc quit ", theme::fg(t.error)),
    ]);
    frame.render_widget(Paragraph::new(footer), footer_area);
}

fn draw_command_popup(frame: &mut Frame, state: &UiState, full_area: Rect) {
    let visible = state.command_matches.len().min(6);
    let width = 52.min(full_area.width);
    let height = ((visible as u16) + 2).min(full_area.height.saturating_sub(4));
    // Anchor just above the bottom of the screen (above prompt+footer).
    let y = full_area.y + full_area.height.saturating_sub(height + 5);
    let popup = Rect { x: full_area.x + 1, y, width, height };

    frame.render_widget(Clear, popup);
    let block = Block::bordered()
        .border_set(border::ROUNDED)
        .border_style(theme::fg(theme::active().accent))
        .title(Span::styled(" commands ", theme::bold(theme::active().accent)))
        .style(Style::default().bg(theme::active().bg));
    frame.render_widget(block, popup);

    let inner = Rect {
        x: popup.x + 1,
        y: popup.y + 1,
        width: popup.width.saturating_sub(2),
        height: popup.height.saturating_sub(2),
    };
    let start = state.command_selected.saturating_sub(visible.saturating_sub(1));
    let lines: Vec<Line> = state
        .command_matches
        .iter()
        .skip(start)
        .take(visible)
        .enumerate()
        .map(|(index, (name, desc))| {
            let is_selected = start + index == state.command_selected;
            let (name_style, desc_style) = if is_selected {
                (theme::bold(theme::active().primary), theme::fg(theme::active().ink))
            } else {
                (theme::fg(theme::active().accent), theme::fg(theme::active().weak))
            };
            Line::from(vec![
                Span::styled(format!(" {name:<26}"), name_style),
                Span::styled(desc.to_string(), desc_style),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

fn draw_sidebar(frame: &mut Frame, area: Rect, state: &UiState, selected: usize, directory: &str) {
    let t = theme::active();
    let [sessions_area, info_area] = Layout::vertical([Constraint::Min(4), Constraint::Length(7)])
        .areas(area);

    let width = sessions_area.width.saturating_sub(8) as usize;
    let items: Vec<ListItem> = state
        .sessions
        .iter()
        .map(|session| {
            ListItem::new(Line::from(vec![
                Span::styled("● ", theme::fg(t.success)),
                Span::styled(truncate(&session.title, width), theme::fg(t.ink)),
                Span::styled(format!(" {}", ago(session.created_at)), theme::fg(t.weak)),
            ]))
        })
        .collect();

    let list_block = Block::bordered()
        .border_set(border::ROUNDED)
        .border_style(theme::fg(t.accent))
        .title(Span::styled(" sessions ", theme::bold(t.accent)));
    let list = List::new(items)
        .block(list_block)
        .highlight_symbol("▌ ")
        .highlight_style(theme::bold(t.primary));
    let mut list_state = ratatui::widgets::ListState::default().with_selected(Some(selected));
    frame.render_stateful_widget(list, sessions_area, &mut list_state);

    // Info block.
    let msg_count = state.sessions.iter().map(|s| s.messages.len()).sum::<usize>();
    let info_block = Block::bordered()
        .border_set(border::ROUNDED)
        .border_style(theme::fg(t.weak))
        .title(Span::styled(" info ", theme::fg(t.weak)));
    let mode_label = if state.plan_mode { "plan" } else { "build" };
    let mode_color = if state.plan_mode { t.warning } else { t.success };
    let info = vec![
        Line::from(vec![
            Span::styled(" mode  ", theme::fg(t.weak)),
            Span::styled(mode_label.to_string(), theme::bold(mode_color)),
        ]),
        Line::from(vec![
            Span::styled(" dir   ", theme::fg(t.weak)),
            Span::styled(shorten_path(directory, 22), theme::fg(t.ink)),
        ]),
        Line::from(vec![
            Span::styled(" ses   ", theme::fg(t.weak)),
            Span::styled(state.sessions.len().to_string(), theme::fg(t.info)),
            Span::styled(" open", theme::fg(t.weak)),
        ]),
        Line::from(vec![
            Span::styled(" msgs  ", theme::fg(t.weak)),
            Span::styled(msg_count.to_string(), theme::fg(t.info)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled(" n ", theme::bold(t.success)),
            Span::styled("new      ", theme::fg(t.weak)),
            Span::styled(" d ", theme::bold(t.error)),
            Span::styled("delete", theme::fg(t.weak)),
        ]),
        Line::from(vec![
            Span::styled(" ↑↓ ", theme::bold(t.info)),
            Span::styled("select   ", theme::fg(t.weak)),
            Span::styled(" ⏎ ", theme::bold(t.primary)),
            Span::styled("open", theme::fg(t.weak)),
        ]),
    ];
    frame.render_widget(info_block, info_area);
    frame.render_widget(
        Paragraph::new(info).style(theme::fg(t.ink)),
        ratatui::widgets::Block::bordered()
            .border_set(border::ROUNDED)
            .inner(info_area),
    );
}

fn draw_help(frame: &mut Frame, area: Rect) {
    let t = theme::active();
    let width = 46.min(area.width);
    let height = 15.min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let popup = Rect { x, y, width, height };

    frame.render_widget(Clear, popup);
    let block = Block::bordered()
        .border_set(border::ROUNDED)
        .border_style(theme::fg(t.primary))
        .title(Span::styled(" keybinds ", theme::bold(t.primary)))
        .style(Style::default().bg(t.bg));
    frame.render_widget(block, popup);

    let rows: [(&str, &str); 11] = [
        ("enter", "send prompt / open session"),
        ("esc", "close panel · quit"),
        ("tab", "toggle sessions sidebar"),
        ("?", "toggle this help"),
        ("/", "slash commands (type in prompt)"),
        ("pgup/pgdn", "scroll transcript"),
        ("↑/↓ j/k", "select session (sidebar)"),
        ("n", "new session (sidebar)"),
        ("d", "delete session (sidebar)"),
        ("ctrl+c", "quit immediately"),
        ("", "press any key to close"),
    ];
    let lines: Vec<Line> = rows
        .iter()
        .map(|(key, desc)| {
            Line::from(vec![
                Span::styled(format!(" {key:<10}"), theme::bold(t.info)),
                Span::styled(*desc, theme::fg(t.ink)),
            ])
        })
        .collect();
    let inner = ratatui::widgets::Block::bordered()
        .border_set(border::ROUNDED)
        .inner(popup);
    frame.render_widget(Paragraph::new(lines), inner);
}

// ---------------------------------------------------------------------------
// helpers

fn message_lines(message: &Message, width: usize) -> Vec<Line<'static>> {
    let t = theme::active();
    let (label, color): (String, _) = match message.role {
        Role::User => ("❯ you".to_string(), t.primary),
        Role::Assistant => (format!("✻ {BRAND}"), t.accent),
    };
    let age = ago(message.created_at);
    let mut lines = vec![Line::from(vec![
        Span::styled(label, theme::bold(color)),
        Span::styled(format!("  · {age}"), theme::fg(t.weak)),
    ])];

    for part in &message.parts {
        match part {
            Part::Text { text } => extend_markdown_lines(text, width.saturating_sub(4), &mut lines),
            Part::ToolUse { name, .. } => lines.push(Line::from(vec![
                Span::styled("  ⚙ tool ", theme::fg(t.info)),
                Span::styled(name.clone(), theme::bold(t.info)),
            ])),
            Part::ToolResult { output, .. } => {
                let first = output.lines().next().unwrap_or_default();
                lines.push(Line::from(Span::styled(
                    format!("  ↳ {}", truncate(first, width.saturating_sub(5))),
                    theme::fg(t.success),
                )));
            }
        }
    }
    lines.push(Line::from(""));
    lines
}

/// Minimal markdown rendering: headings, bullets, numbered lists, code fences
/// and block quotes get upstream-style colors; everything else plain ink.
fn extend_markdown_lines(text: &str, width: usize, out: &mut Vec<Line<'static>>) {
    let mut in_code = false;
    for raw in text.split('\n') {
        let wrapped = wrap_text(raw, width);
        for line in wrapped {
            let styled = style_markdown_line(&line, in_code);
            in_code = styled.1;
            out.push(styled.0);
        }
    }
}

type StyledLine = (Line<'static>, bool);

fn style_markdown_line(line: &str, was_in_code: bool) -> StyledLine {
    let t = theme::active();
    let indent = "  ";
    let trimmed = line.trim_start();

    if trimmed.starts_with("```") {
        return (
            Line::from(Span::styled(format!("{indent}{line}"), theme::fg(t.weak))),
            !was_in_code,
        );
    }
    if was_in_code {
        return (
            Line::from(Span::styled(
                format!("{indent}{line}"),
                theme::fg(t.success),
            )),
            true,
        );
    }
    if trimmed.starts_with('#') {
        return (
            Line::from(Span::styled(format!("{indent}{line}"), theme::bold(t.accent))),
            false,
        );
    }
    if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
        return (
            Line::from(vec![
                Span::styled(format!("{indent}  • "), theme::fg(t.primary)),
                Span::styled(trimmed[2..].to_string(), theme::fg(t.ink)),
            ]),
            false,
        );
    }
    if trimmed.starts_with("> ") {
        return (
            Line::from(Span::styled(
                format!("{indent}  ┃ {line}"),
                theme::fg(t.warning),
            )),
            false,
        );
    }
    let numbered = trimmed
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_digit())
        && trimmed.contains(". ");
    if numbered {
        return (
            Line::from(Span::styled(format!("{indent}{line}"), theme::fg(t.info))),
            false,
        );
    }
    (
        Line::from(Span::styled(format!("{indent}{line}"), theme::fg(t.ink))),
        false,
    )
}

/// Greedy word-wrap producing display lines no wider than `width` chars.
pub fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let mut out = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut current = String::new();
        for word in paragraph.split(' ') {
            let candidate_len = current.chars().count()
                + word.chars().count()
                + usize::from(!current.is_empty());
            if candidate_len > width && !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
        }
        out.push(current);
    }
    out
}

/// Truncate with ellipsis.
pub fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let keep = max.saturating_sub(1);
    format!("{}…", text.chars().take(keep).collect::<String>())
}

pub fn shorten_path(path: &str, max: usize) -> String {
    let replaced = path
        .replace(std::env::var("USERPROFILE").unwrap_or_default().as_str(), "~")
        .replace(std::env::var("HOME").unwrap_or_default().as_str(), "~");
    truncate(&replaced, max)
}

/// Compact relative timestamp like upstream (`now`, `5m`, `2h`, `3d`).
pub fn ago(created_at: chrono::DateTime<chrono::Utc>) -> String {
    let secs = chrono::Utc::now()
        .signed_duration_since(created_at)
        .num_seconds()
        .max(0);
    if secs < 60 {
        "now".to_string()
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_long_text_on_word_boundaries() {
        let wrapped = wrap_text("the quick brown fox jumps over the lazy dog", 15);
        assert!(wrapped.iter().all(|line| line.chars().count() <= 15));
        assert_eq!(wrapped.join(" "), "the quick brown fox jumps over the lazy dog");
    }

    #[test]
    fn preserves_empty_paragraphs_as_blank_lines() {
        assert_eq!(wrap_text("\n\nx", 10), vec!["", "", "x"]);
    }

    #[test]
    fn truncates_with_ellipsis() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 8), "hello w…");
    }

    #[test]
    fn headings_bullets_and_code_get_distinct_treatment() {
        let mut lines = Vec::new();
        extend_markdown_lines("# Title\n- item one\n```\ncode()\n```", 40, &mut lines);
        assert_eq!(lines.len(), 5);
        let texts: Vec<String> = lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.to_string()).collect())
            .collect();
        assert!(texts[0].contains("# Title"));
        assert!(texts[1].contains("• item one"));
        assert!(texts[3].contains("code()"));
    }

    #[test]
    fn ago_buckets() {
        let now = chrono::Utc::now();
        assert_eq!(ago(now), "now");
        assert_eq!(ago(now - chrono::Duration::seconds(120)), "2m");
        assert_eq!(ago(now - chrono::Duration::hours(3)), "3h");
        assert_eq!(ago(now - chrono::Duration::days(2)), "2d");
    }

    #[test]
    fn command_filtering_prefixes_names_only() {
        let mut state = UiState {
            input: "/m".into(),
            scroll_back: 0,
            sidebar: SidebarState::Closed,
            help: HelpState::Hidden,
            sessions: vec![],
            status: StreamStatus::Idle,
            error: None,
            notice: None,
            streaming_message: None,
            command_matches: vec![],
            command_selected: 0,
            plan_mode: false,
            model_spec: "test/m".into(),
        };
        state.update_command_matches();
        let names: Vec<&str> = state.command_matches.iter().map(|(n, _)| n.split(' ').next().unwrap()).collect();
        assert_eq!(names, vec!["/model"]);
        assert!(!state.command_matches.is_empty());

        state.input = "/s".into();
        state.update_command_matches();
        let names: Vec<&str> = state.command_matches.iter().map(|(n, _)| n.split(' ').next().unwrap()).collect();
        assert_eq!(names, vec!["/sessions"]);
    }
}
