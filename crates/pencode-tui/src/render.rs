//! Rendering: opencode-style layout with brand header, transcript pane,
//! right-side sessions sidebar, rounded prompt input, status bar and a
//! help overlay.

use super::theme;
use pencode_protocol::{Message, Part, Role};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::symbols::border;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, Paragraph};
use ratatui::Frame;

const BRAND: &str = "pencode";
const VERSION: &str = env!("CARGO_PKG_VERSION");

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

pub struct UiState {
    pub input: String,
    /// Lines scrolled back from the newest message (0 = pinned to bottom).
    pub scroll_back: usize,
    pub sidebar: SidebarState,
    pub help: HelpState,
    pub sessions: Vec<pencode_protocol::Session>,
}

impl UiState {
    pub fn refresh_sessions(&mut self, store: &pencode_core::session::Store) -> anyhow::Result<()> {
        self.sessions = store.list()?;
        Ok(())
    }
}

pub fn draw(
    frame: &mut Frame,
    session: &pencode_protocol::Session,
    state: &UiState,
    model: &str,
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

    draw_main(frame, main_area, session, state, model, directory);

    if let (Some(side), SidebarState::Open { selected }) = (sidebar_area, state.sidebar) {
        draw_sidebar(frame, side, state, selected, directory);
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
    model: &str,
    directory: &str,
) {
    let [header_area, transcript_area, input_area, footer_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
    .areas(area);

    // Header: brand + session title + working directory.
    let dir_label = shorten_path(directory, 32);
    let title_width = area
        .width
        .saturating_sub(dir_label.chars().count() as u16 + 24)
        .max(8) as usize;
    let header = Line::from(vec![
        Span::styled(" ◆ ", theme::bold(theme::PRIMARY)),
        Span::styled(BRAND, theme::bold(theme::PRIMARY)),
        Span::styled("  │  ", theme::fg(theme::WEAK)),
        Span::styled(truncate(&session.title, title_width), theme::fg(theme::INK)),
        Span::styled(format!("  ({dir_label})"), theme::fg(theme::WEAK)),
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
            "  type a prompt below and press enter",
            theme::bold(theme::PRIMARY),
        )));
        lines.push(Line::from(Span::styled(
            "  messages persist to the local session store",
            theme::fg(theme::WEAK),
        )));
        lines.push(Line::from(Span::styled(
            "  press tab for sessions · ? for help",
            theme::fg(theme::WEAK),
        )));
    }

    let total = lines.len();
    let max_scroll = total.saturating_sub(inner_height);
    let offset = max_scroll.saturating_sub(state.scroll_back.min(max_scroll));
    let transcript_block = Block::bordered()
        .border_set(border::ROUNDED)
        .border_style(theme::fg(theme::WEAK));
    frame.render_widget(transcript_block, transcript_area);
    frame.render_widget(
        Paragraph::new(lines)
            .style(Style::default().bg(theme::BG))
            .scroll((offset as u16, 0)),
        transcript_area,
    );

    // Prompt input with rounded border and visible cursor.
    let input_block = Block::bordered()
        .border_set(border::ROUNDED)
        .border_style(if state.input.is_empty() {
            theme::fg(theme::WEAK)
        } else {
            theme::fg(theme::PRIMARY)
        })
        .title(Span::styled(" prompt ", theme::fg(theme::PRIMARY)));
    frame.render_widget(input_block, input_area);
    frame.render_widget(
        Paragraph::new(state.input.as_str()).style(theme::fg(theme::INK)),
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
        Span::styled(format!(" {model} "), theme::fg(theme::INFO)),
        Span::styled("│", theme::fg(theme::WEAK)),
        Span::styled(format!(" v{VERSION} "), theme::fg(theme::WEAK)),
        Span::styled("│", theme::fg(theme::WEAK)),
        Span::styled(format!(" {short_session} "), theme::fg(theme::WEAK)),
        Span::styled("│", theme::fg(theme::WEAK)),
        Span::styled(" tab sessions ", theme::fg(theme::ACCENT)),
        Span::styled("·", theme::fg(theme::WEAK)),
        Span::styled(" ? help ", theme::fg(theme::ACCENT)),
        Span::styled("·", theme::fg(theme::WEAK)),
        Span::styled(" enter send ", theme::fg(theme::SUCCESS)),
        Span::styled("·", theme::fg(theme::WEAK)),
        Span::styled(" esc quit ", theme::fg(theme::ERROR)),
    ]);
    frame.render_widget(Paragraph::new(footer), footer_area);
}

fn draw_sidebar(frame: &mut Frame, area: Rect, state: &UiState, selected: usize, directory: &str) {
    let [sessions_area, info_area] = Layout::vertical([Constraint::Min(4), Constraint::Length(7)])
        .areas(area);

    // Sessions list.
    let width = sessions_area.width.saturating_sub(6) as usize;
    let items: Vec<ListItem> = state
        .sessions
        .iter()
        .map(|session| {
            let marker = Span::styled("● ", theme::fg(theme::SUCCESS));
            ListItem::new(Line::from(vec![
                marker,
                Span::styled(truncate(&session.title, width), theme::fg(theme::INK)),
                Span::styled(format!(" {}", ago(session.created_at)), theme::fg(theme::WEAK)),
            ]))
        })
        .collect();

    let list_block = Block::bordered()
        .border_set(border::ROUNDED)
        .border_style(theme::fg(theme::ACCENT))
        .title(Span::styled(" sessions ", theme::bold(theme::ACCENT)));
    let list = List::new(items)
        .block(list_block)
        .highlight_symbol("▌ ")
        .highlight_style(theme::bold(theme::PRIMARY));
    let mut list_state = ratatui::widgets::ListState::default().with_selected(Some(selected));
    frame.render_stateful_widget(list, sessions_area, &mut list_state);

    // Info block.
    let msg_count = state
        .sessions
        .iter()
        .map(|s| s.messages.len())
        .sum::<usize>();
    let info_block = Block::bordered()
        .border_set(border::ROUNDED)
        .border_style(theme::fg(theme::WEAK))
        .title(Span::styled(" info ", theme::fg(theme::WEAK)));
    let info = vec![
        Line::from(vec![
            Span::styled(" dir   ", theme::fg(theme::WEAK)),
            Span::styled(shorten_path(directory, 22), theme::fg(theme::INK)),
        ]),
        Line::from(vec![
            Span::styled(" ses   ", theme::fg(theme::WEAK)),
            Span::styled(state.sessions.len().to_string(), theme::fg(theme::INFO)),
            Span::styled(" open", theme::fg(theme::WEAK)),
        ]),
        Line::from(vec![
            Span::styled(" msgs  ", theme::fg(theme::WEAK)),
            Span::styled(msg_count.to_string(), theme::fg(theme::INFO)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled(" n ", theme::bold(theme::SUCCESS)),
            Span::styled("new      ", theme::fg(theme::WEAK)),
            Span::styled(" d ", theme::bold(theme::ERROR)),
            Span::styled("delete", theme::fg(theme::WEAK)),
        ]),
        Line::from(vec![
            Span::styled(" ↑↓ ", theme::bold(theme::INFO)),
            Span::styled("select   ", theme::fg(theme::WEAK)),
            Span::styled(" ⏎ ", theme::bold(theme::PRIMARY)),
            Span::styled("open", theme::fg(theme::WEAK)),
        ]),
    ];
    frame.render_widget(info_block, info_area);
    frame.render_widget(
        Paragraph::new(info).style(theme::fg(theme::INK)),
        ratatui::widgets::Block::bordered()
            .border_set(border::ROUNDED)
            .inner(info_area),
    );
}

fn draw_help(frame: &mut Frame, area: Rect) {
    let width = 46.min(area.width);
    let height = 14.min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let popup = Rect { x, y, width, height };

    frame.render_widget(Clear, popup);
    let block = Block::bordered()
        .border_set(border::ROUNDED)
        .border_style(theme::fg(theme::PRIMARY))
        .title(Span::styled(" keybinds ", theme::bold(theme::PRIMARY)))
        .style(Style::default().bg(theme::BG));
    frame.render_widget(block, popup);

    let rows: [(&str, &str); 10] = [
        ("enter", "send prompt / open session"),
        ("esc", "close panel · quit"),
        ("tab", "toggle sessions sidebar"),
        ("?", "toggle this help"),
        ("pgup/pgdn", "scroll transcript"),
        ("↑/↓ j/k", "select session (sidebar)"),
        ("n", "new session (sidebar)"),
        ("d", "delete session (sidebar)"),
        ("ctrl+c", "quit immediately"),
        ("", ""),
    ];
    let lines: Vec<Line> = rows
        .iter()
        .map(|(key, desc)| {
            Line::from(vec![
                Span::styled(format!(" {key:<10}"), theme::bold(theme::INFO)),
                Span::styled(*desc, theme::fg(theme::INK)),
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
    let (label, color): (String, _) = match message.role {
        Role::User => ("❯ you".to_string(), theme::PRIMARY),
        Role::Assistant => (format!("✻ {BRAND}"), theme::ACCENT),
    };
    let age = ago(message.created_at);
    let mut lines = vec![Line::from(vec![
        Span::styled(label, theme::bold(color)),
        Span::styled(format!("  · {age}"), theme::fg(theme::WEAK)),
    ])];

    for part in &message.parts {
        match part {
            Part::Text { text } => extend_markdown_lines(text, width.saturating_sub(4), &mut lines),
            Part::ToolUse { name, .. } => lines.push(Line::from(vec![
                Span::styled("  ⚙ tool ", theme::fg(theme::INFO)),
                Span::styled(name.clone(), theme::bold(theme::INFO)),
            ])),
            Part::ToolResult { output, .. } => {
                let first = output.lines().next().unwrap_or_default();
                lines.push(Line::from(Span::styled(
                    format!("  ↳ {}", truncate(first, width.saturating_sub(5))),
                    theme::fg(theme::SUCCESS),
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
        for (index, line) in wrapped.into_iter().enumerate() {
            let continuation = index > 0 || in_code;
            let styled = style_markdown_line(&line, in_code, continuation);
            in_code = styled.1;
            out.push(styled.0);
        }
    }
}

type StyledLine = (Line<'static>, bool);

fn style_markdown_line(line: &str, was_in_code: bool, _continuation: bool) -> StyledLine {
    let indent = "  ";
    let trimmed = line.trim_start();

    if trimmed.starts_with("```") {
        return (
            Line::from(Span::styled(
                format!("{indent}{line}"),
                theme::fg(theme::WEAK),
            )),
            !was_in_code,
        );
    }
    if was_in_code {
        return (
            Line::from(Span::styled(
                format!("{indent}{line}"),
                theme::fg(theme::SUCCESS),
            )),
            true,
        );
    }
    if trimmed.starts_with('#') {
        return (
            Line::from(Span::styled(
                format!("{indent}{line}"),
                theme::bold(theme::ACCENT),
            )),
            false,
        );
    }
    if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
        return (
            Line::from(vec![
                Span::styled(format!("{indent}  • "), theme::fg(theme::PRIMARY)),
                Span::styled(trimmed[2..].to_string(), theme::fg(theme::INK)),
            ]),
            false,
        );
    }
    if trimmed.starts_with("> ") {
        return (
            Line::from(Span::styled(
                format!("{indent}  ┃ {line}"),
                theme::fg(theme::WARNING),
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
            Line::from(Span::styled(
                format!("{indent}{line}"),
                theme::fg(theme::INFO),
            )),
            false,
        );
    }
    (
        Line::from(Span::styled(
            format!("{indent}{line}"),
            theme::fg(theme::INK),
        )),
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

/// `ses_abc123…` → `ses_abc123`, `~/very/long/path` → shortened form.
pub fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let keep = max.saturating_sub(1);
    format!("{}…", text.chars().take(keep).collect::<String>())
}

pub fn shorten_path(path: &str, max: usize) -> String {
    let replaced = path.replace(std::env::var("USERPROFILE").unwrap_or_default().as_str(), "~")
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
}
