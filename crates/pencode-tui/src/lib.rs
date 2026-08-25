//! Terminal UI for pencode, modeled on the upstream opencode TUI:
//! brand header, transcript with role-labelled message blocks, markdown-ish
//! rendering, a right-side sessions sidebar (tab), help overlay (?),
//! rounded prompt input and a status bar.
//!
//! Keys: `enter` send · `esc` quit/close · `tab` sidebar · `?` help
//!       `pgup`/`pgdn` scroll · sidebar: `↑/↓` select, `enter` open,
//!       `n` new session, `d` delete session

pub mod theme;
pub mod render;

use anyhow::Context;
use pencode_core::App;
use pencode_protocol::{Message, Part, Role};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::Stdout;

use render::{draw, HelpState, SidebarState, UiState};

pub fn run(app: App) -> anyhow::Result<()> {
    let store = app.store().clone();
    let model = app
        .config()
        .model
        .clone()
        .unwrap_or_else(|| "(model not configured)".to_string());
    let directory = std::env::current_dir()
        .map(|dir| dir.display().to_string())
        .unwrap_or_else(|_| ".".to_string());

    // Reuse the most recent session when one exists, like upstream does.
    let mut session = match store.list()?.into_iter().next() {
        Some(existing) => existing,
        None => store.create(&directory)?,
    };
    let sessions = store.list()?;

    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(
        stdout,
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut state = UiState {
        input: String::new(),
        scroll_back: 0,
        sidebar: if sessions.len() > 1 {
            SidebarState::Open { selected: 0 }
        } else {
            SidebarState::Closed
        },
        help: HelpState::Hidden,
        sessions,
    };

    let result = event_loop(
        &mut terminal,
        &mut session,
        &mut state,
        &store,
        &model,
        &directory,
    );

    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        std::io::stdout(),
        crossterm::event::DisableMouseCapture,
        crossterm::terminal::LeaveAlternateScreen
    )?;
    result
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    session: &mut pencode_protocol::Session,
    state: &mut UiState,
    store: &pencode_core::session::Store,
    model: &str,
    directory: &str,
) -> anyhow::Result<()> {
    loop {
        terminal.draw(|frame| draw(frame, session, state, model, directory))?;

        if !crossterm::event::poll(std::time::Duration::from_millis(100))? {
            continue;
        }
        match crossterm::event::read()? {
            crossterm::event::Event::Key(key)
                if key.kind == crossterm::event::KeyEventKind::Press =>
            {
                use crossterm::event::{KeyCode, KeyModifiers};

                // Help overlay swallows the next key and closes.
                if matches!(state.help, HelpState::Visible) {
                    state.help = HelpState::Hidden;
                    continue;
                }

                match key.code {
                    KeyCode::Esc => {
                        if matches!(state.sidebar, SidebarState::Open { .. }) {
                            state.sidebar = SidebarState::Closed;
                        } else {
                            return Ok(());
                        }
                    }
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(())
                    }
                    KeyCode::Char('?') => {
                        state.help = HelpState::Visible;
                    }
                    KeyCode::Tab => match state.sidebar {
                        SidebarState::Closed => {
                            state.refresh_sessions(store)?;
                            state.sidebar = SidebarState::Open { selected: 0 };
                        }
                        SidebarState::Open { .. } => {
                            state.sidebar = SidebarState::Closed;
                        }
                    },
                    KeyCode::PageUp => state.scroll_back = state.scroll_back.saturating_add(10),
                    KeyCode::PageDown => {
                        state.scroll_back = state.scroll_back.saturating_sub(10)
                    }
                    KeyCode::Backspace => {
                        state.input.pop();
                    }
                    KeyCode::Enter => {
                        if let SidebarState::Open { selected } = state.sidebar {
                            // Sidebar focused: enter opens the highlighted session.
                            if let Some(target) = state.sessions.get(selected).map(|s| s.id.clone())
                            {
                                *session = store.get(&target)?;
                                state.scroll_back = 0;
                                state.sidebar = SidebarState::Closed;
                            }
                        } else if !state.input.trim().is_empty() {
                            let text = std::mem::take(&mut state.input);
                            state.scroll_back = 0;
                            let message = Message::new(Role::User, vec![Part::text(text)]);
                            session.push(message);
                            store.save(session)?;
                            // TODO: route through the model provider once wired up;
                            // assistant replies currently arrive via the server API.
                        }
                    }
                    KeyCode::Up | KeyCode::Char('k')
                        if matches!(state.sidebar, SidebarState::Open { .. }) =>
                    {
                        if let SidebarState::Open { selected } = state.sidebar {
                            let next = selected.saturating_sub(1);
                            state.sidebar = SidebarState::Open { selected: next };
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j')
                        if matches!(state.sidebar, SidebarState::Open { .. }) =>
                    {
                        if let SidebarState::Open { selected } = state.sidebar {
                            let next = (selected + 1).min(state.sessions.len().saturating_sub(1));
                            state.sidebar = SidebarState::Open { selected: next };
                        }
                    }
                    KeyCode::Char('n') if matches!(state.sidebar, SidebarState::Open { .. }) => {
                        *session = store.create(directory)?;
                        state.refresh_sessions(store)?;
                        state.sidebar = SidebarState::Closed;
                        state.scroll_back = 0;
                    }
                    KeyCode::Char('d') if matches!(state.sidebar, SidebarState::Open { .. }) => {
                        if let SidebarState::Open { selected } = state.sidebar {
                            if let Some(target) = state.sessions.get(selected).map(|s| s.id.clone())
                            {
                                if target != session.id && state.sessions.len() > 1 {
                                    store.remove(&target)?;
                                    state.refresh_sessions(store)?;
                                    let len = state.sessions.len();
                                    state.sidebar = SidebarState::Open {
                                        selected: selected.min(len.saturating_sub(1)),
                                    };
                                }
                            }
                        }
                    }
                    KeyCode::Char(ch) => {
                        state.input.push(ch);
                        state.scroll_back = 0;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

/// Kept for future non-TUI fallback rendering.
#[allow(dead_code)]
fn flush_stdout(stdout: &mut Stdout) -> anyhow::Result<()> {
    use std::io::Write;
    stdout.flush().context("flushing stdout")
}
