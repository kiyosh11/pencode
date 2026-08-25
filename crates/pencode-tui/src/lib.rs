//! Terminal UI for pencode, modeled on the upstream opencode TUI:
//! brand header, transcript with role-labelled message blocks, markdown-ish
//! rendering, right-side sessions sidebar, slash commands with autocomplete,
//! help overlay, themes, agent modes and streaming provider replies.
//!
//! Keys: `enter` send · `esc` quit/close · `tab` sidebar/complete · `?` help
//!       `/` commands · `pgup`/`pgdn` scroll
//!       sidebar: `↑/↓` select, `enter` open, `n` new session, `d` delete

pub mod render;
pub mod theme;

use anyhow::Context;
use pencode_core::App;
use pencode_provider::{self, Prompt};
use pencode_protocol::{Message, Part, Role};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::Stdout;

use render::{draw, HelpState, SidebarState, StreamStatus, UiState};

pub fn run(app: App) -> anyhow::Result<()> {
    let store = app.store().clone();
    let config = app.config().clone();
    let model_spec = config
        .model
        .clone()
        .unwrap_or_else(|| "anthropic/claude-sonnet-4-5".to_string());
    theme::set_dark();

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
        sidebar: SidebarState::Closed,
        help: HelpState::Hidden,
        sessions,
        status: StreamStatus::Idle,
        error: None,
        notice: None,
        streaming_message: None,
        command_matches: vec![],
        command_selected: 0,
        plan_mode: false,
        model_spec,
    };

    let result = event_loop(
        &mut terminal,
        &mut session,
        &mut state,
        &store,
        &directory,
        &config,
    );

    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        std::io::stdout(),
        crossterm::event::DisableMouseCapture,
        crossterm::terminal::LeaveAlternateScreen
    )?;
    result
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingAction {
    Reply,
    Compact,
    Init,
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    session: &mut pencode_protocol::Session,
    state: &mut UiState,
    store: &pencode_core::session::Store,
    directory: &str,
    config: &pencode_core::config::Config,
) -> anyhow::Result<()> {
    let (tx, rx) = std::sync::mpsc::channel::<StreamEvent>();
    let mut pending_action = PendingAction::Reply;
    let mut assistant_index: Option<usize> = None;

    // Drain helper run on every tick.
    macro_rules! drain_events {
        () => {
            while let Ok(event) = rx.try_recv() {
                match event {
                    StreamEvent::Delta(text) => {
                        state.status = StreamStatus::Streaming;
                        if pending_action == PendingAction::Compact
                            || pending_action == PendingAction::Init
                        {
                            continue;
                        }
                        let index = match assistant_index {
                            Some(index) => index,
                            None => {
                                let message =
                                    Message::new(Role::Assistant, Vec::new());
                                session.push(message);
                                let index = session.messages.len() - 1;
                                assistant_index = Some(index);
                                index
                            }
                        };
                        append_text(&mut session.messages[index], &text);
                    }
                    StreamEvent::Finished(Ok(full)) => match pending_action {
                        PendingAction::Compact => {
                            let summary_message = Message::new(
                                Role::Assistant,
                                vec![Part::text(format!(
                                    "[compacted summary of previous conversation]\n\n{full}"
                                ))],
                            );
                            *session = pencode_protocol::Session {
                                id: session.id.clone(),
                                title: format!("compacted: {}", truncate_title(&full)),
                                directory: session.directory.clone(),
                                created_at: session.created_at,
                                messages: vec![summary_message],
                            };
                            store.save(session)?;
                            state.refresh_sessions(store)?;
                            state.notice = Some("session compacted".into());
                        }
                        PendingAction::Init => {
                            let path = std::path::Path::new("AGENTS.md");
                            if path.exists() {
                                state.error =
                                    Some("AGENTS.md already exists; not overwriting".into());
                            } else {
                                std::fs::write(path, &full)
                                    .context("writing AGENTS.md")?;
                                state.notice = Some("wrote AGENTS.md".into());
                            }
                        }
                        PendingAction::Reply => {
                            assistant_index = None;
                            store.save(session)?;
                        }
                    },
                    StreamEvent::Finished(Err(err)) => {
                        if pending_action == PendingAction::Reply {
                            // Drop a half-written empty assistant bubble.
                            if let Some(index) = assistant_index {
                                if index < session.messages.len()
                                    && session.messages[index].text().is_empty()
                                {
                                    session.messages.remove(index);
                                }
                            }
                        }
                        assistant_index = None;
                        state.error = Some(render::truncate(&err.to_string().replace('\n', " "), 120));
                    }
                }
                state.streaming_message = None;
                state.status = StreamStatus::Idle;
            }
        };
    }

    loop {
        drain_events!();

        terminal.draw(|frame| draw(frame, session, state, directory))?;

        if !crossterm::event::poll(std::time::Duration::from_millis(100))? {
            continue;
        }
        match crossterm::event::read()? {
            crossterm::event::Event::Key(key)
                if key.kind == crossterm::event::KeyEventKind::Press =>
            {
                use crossterm::event::{KeyCode, KeyModifiers};

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
                    KeyCode::Char('?') if state.input.is_empty() => {
                        state.help = HelpState::Visible;
                    }
                    KeyCode::Tab => {
                        // Complete a slash command first, else toggle sidebar.
                        if !state.command_matches.is_empty() && state.input.starts_with('/') {
                            state.accept_command();
                        } else {
                            match state.sidebar {
                                SidebarState::Closed => {
                                    state.refresh_sessions(store)?;
                                    state.sidebar = SidebarState::Open { selected: 0 };
                                }
                                SidebarState::Open { .. } => {
                                    state.sidebar = SidebarState::Closed;
                                }
                            }
                        }
                    }
                    KeyCode::PageUp => state.scroll_back = state.scroll_back.saturating_add(10),
                    KeyCode::PageDown => {
                        state.scroll_back = state.scroll_back.saturating_sub(10)
                    }
                    KeyCode::Backspace => {
                        state.input.pop();
                        state.update_command_matches();
                    }
                    KeyCode::Up if !state.command_matches.is_empty() => {
                        state.command_selected = state.command_selected.saturating_sub(1);
                    }
                    KeyCode::Down if !state.command_matches.is_empty() => {
                        state.command_selected =
                            (state.command_selected + 1).min(state.command_matches.len() - 1);
                    }
                    KeyCode::Enter => {
                        if let SidebarState::Open { selected } = state.sidebar {
                            if let Some(target) = state.sessions.get(selected).map(|s| s.id.clone())
                            {
                                *session = store.get(&target)?;
                                assistant_index = None;
                                state.scroll_back = 0;
                                state.sidebar = SidebarState::Closed;
                            }
                        } else if !state.input.trim().is_empty() {
                            // Complete highlighted command unless fully typed.
                            let typed = state.input.trim().to_string();
                            let head = typed.split(' ').next().unwrap_or("").to_lowercase();
                            if !state.command_matches.is_empty() {
                                let exact = state.command_matches.iter().any(|(name, _)| {
                                    name.split(' ').next() == Some(head.as_str())
                                });
                                if !exact {
                                    state.accept_command();
                                    continue;
                                }
                            }

                            let text = std::mem::take(&mut state.input);
                            state.command_matches.clear();
                            state.scroll_back = 0;

                            if let Some(command) = text.strip_prefix('/') {
                                let mut parts = command.splitn(2, ' ');
                                let name = parts.next().unwrap_or("").trim();
                                let args = parts.next().unwrap_or("").trim().to_string();
                                match execute_command(
                                    name,
                                    &args,
                                    state,
                                    session,
                                    store,
                                    directory,
                                    config,
                                )? {
                                    CommandOutcome::Quit => return Ok(()),
                                    CommandOutcome::Spawn(resolved, prompt, action) => {
                                        pending_action = action;
                                        state.status = StreamStatus::Thinking;
                                        state.error = None;
                                        state.notice = None;
                                        state.streaming_message = Some(session.id.clone());
                                        spawn_completion(tx.clone(), resolved, prompt);
                                    }
                                    CommandOutcome::Handled => {}
                                }
                                continue;
                            }

                            let message = Message::new(Role::User, vec![Part::text(text)]);
                            session.push(message);
                            store.save(session)?;

                            match pencode_provider::resolve(&state.model_spec, config) {
                                Ok(resolved) => {
                                    let mut prompt = Prompt::from_session(session);
                                    prompt.system = Some(system_prompt(state.plan_mode));
                                    pending_action = PendingAction::Reply;
                                    state.status = StreamStatus::Thinking;
                                    state.error = None;
                                    state.notice = None;
                                    state.streaming_message = Some(session.id.clone());
                                    spawn_completion(tx.clone(), resolved, prompt);
                                }
                                Err(err) => {
                                    state.error = Some(render::truncate(
                                        &err.to_string().replace('\n', " "),
                                        120,
                                    ));
                                }
                            }
                        }
                    }
                    KeyCode::Up | KeyCode::Char('k')
                        if matches!(state.sidebar, SidebarState::Open { .. })
                            && state.command_matches.is_empty() =>
                    {
                        if let SidebarState::Open { selected } = state.sidebar {
                            let next = selected.saturating_sub(1);
                            state.sidebar = SidebarState::Open { selected: next };
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j')
                        if matches!(state.sidebar, SidebarState::Open { .. })
                            && state.command_matches.is_empty() =>
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
                        state.update_command_matches();
                        state.scroll_back = 0;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// slash commands

enum CommandOutcome {
    Handled,
    Quit,
    Spawn(pencode_provider::Resolved, Prompt, PendingAction),
}

fn execute_command(
    name: &str,
    args: &str,
    state: &mut UiState,
    session: &mut pencode_protocol::Session,
    store: &pencode_core::session::Store,
    directory: &str,
    config: &pencode_core::config::Config,
) -> anyhow::Result<CommandOutcome> {
    match name {
        "help" => state.help = render::HelpState::Visible,
        "exit" | "quit" | "q" => return Ok(CommandOutcome::Quit),
        "new" | "clear" => {
            *session = store.create(directory)?;
            state.refresh_sessions(store)?;
            state.scroll_back = 0;
            state.notice = Some("started a new session".into());
        }
        "sessions" => {
            state.refresh_sessions(store)?;
            state.sidebar = match state.sidebar {
                SidebarState::Closed => SidebarState::Open { selected: 0 },
                SidebarState::Open { .. } => SidebarState::Closed,
            };
        }
        "model" | "models" => {
            if args.is_empty() {
                state.notice = Some(format!(
                    "model: {} — usage: /model <provider/model>",
                    state.model_spec
                ));
            } else if args.split('/').count() >= 2 {
                state.model_spec = args.to_string();
                state.notice = Some(format!("model set to {args}"));
            } else {
                state.error = Some("expected /model <provider/model>".into());
            }
        }
        "themes" => {
            if theme::is_dark() {
                theme::set_light();
                state.notice = Some("theme: light".into());
            } else {
                theme::set_dark();
                state.notice = Some("theme: dark".into());
            }
        }
        "agent" => {
            state.plan_mode = !state.plan_mode;
            let mode = if state.plan_mode { "plan" } else { "build" };
            state.notice = Some(format!("agent mode: {mode}"));
        }
        "compact" => {
            let mut prompt = Prompt::from_session(session);
            prompt.system = Some(
                "Summarize the conversation so far into a compact brief. \
                 Preserve key decisions, file paths, identifiers and open todos."
                    .into(),
            );
            let resolved = match pencode_provider::resolve(&state.model_spec, config) {
                Ok(resolved) => resolved,
                Err(err) => {
                    state.error = Some(render::truncate(&err.to_string(), 120));
                    return Ok(CommandOutcome::Handled);
                }
            };
            return Ok(CommandOutcome::Spawn(resolved, prompt, PendingAction::Compact));
        }
        "undo" => {
            let removed_assistant = session.messages.last().is_some_and(|m| m.role == Role::Assistant);
            if removed_assistant {
                session.messages.pop();
            }
            let popped_user = session.messages.last().is_some_and(|m| m.role == Role::User);
            if popped_user {
                session.messages.pop();
            }
            store.save(session)?;
            state.notice = Some(if popped_user || removed_assistant {
                "removed last exchange".into()
            } else {
                "nothing to undo".into()
            });
        }
        "retry" => {
            // Drop trailing assistant reply, keep/re-add the last user turn.
            if session.messages.last().is_some_and(|m| m.role == Role::Assistant) {
                session.messages.pop();
            }
            let last_user = session
                .messages
                .last()
                .filter(|m| m.role == Role::User)
                .cloned();
            match last_user {
                Some(user_message) => {
                    session.messages.pop();
                    let resent = Message::new(Role::User, vec![Part::text(user_message.text())]);
                    session.push(resent);
                    store.save(session)?;
                    let mut prompt = Prompt::from_session(session);
                    prompt.system = Some(system_prompt(state.plan_mode));
                    let resolved = match pencode_provider::resolve(&state.model_spec, config) {
                        Ok(resolved) => resolved,
                        Err(err) => {
                            state.error = Some(render::truncate(&err.to_string(), 120));
                            return Ok(CommandOutcome::Handled);
                        }
                    };
                    state.status = StreamStatus::Thinking;
                    state.error = None;
                    state.notice = None;
                    return Ok(CommandOutcome::Spawn(resolved, prompt, PendingAction::Reply));
                }
                None => {
                    state.notice = Some("nothing to retry".into());
                }
            }
        }
        "init" => {
            if std::path::Path::new("AGENTS.md").exists() {
                state.notice = Some("AGENTS.md already exists".into());
            } else {
                let prompt = Prompt {
                    system: Some(
                        "You are pencode, a coding agent. Produce the contents of an \
                         AGENTS.md file for this repository: concise guidance covering \
                         build commands, architecture overview and conventions."
                            .into(),
                    ),
                    messages: vec![(
                        Role::User,
                        format!(
                            "Generate AGENTS.md content for the repository at {directory}."
                        ),
                    )],
                };
                let resolved = match pencode_provider::resolve(&state.model_spec, config) {
                    Ok(resolved) => resolved,
                    Err(err) => {
                        state.error = Some(render::truncate(&err.to_string(), 120));
                        return Ok(CommandOutcome::Handled);
                    }
                };
                state.status = StreamStatus::Thinking;
                state.error = None;
                state.notice = None;
                return Ok(CommandOutcome::Spawn(resolved, prompt, PendingAction::Init));
            }
        }
        "version" => {
            state.notice = Some(format!("pencode v{} (rust)", render::VERSION));
        }
        other => {
            state.error = Some(format!("unknown command /{other} — try /help"));
        }
    }
    Ok(CommandOutcome::Handled)
}

fn system_prompt(plan_mode: bool) -> String {
    let mut base = String::from(
        "You are pencode, a terminal-based coding agent. Be concise and precise. \
         Use markdown formatting where helpful.",
    );
    if plan_mode {
        base.push_str(
            " The user is in PLAN mode: analyze and advise only — do not modify \
             any files or run mutating commands.",
        );
    }
    base
}

fn truncate_title(text: &str) -> String {
    render::truncate(text.split('\n').next().unwrap_or_default(), 40)
}
// ---------------------------------------------------------------------------
// provider streaming

enum StreamEvent {
    Delta(String),
    Finished(anyhow::Result<String>),
}

fn spawn_completion(
    tx: std::sync::mpsc::Sender<StreamEvent>,
    resolved: pencode_provider::Resolved,
    prompt: Prompt,
) {
    let _ = std::thread::spawn(move || {
        let result = pencode_provider::stream(&resolved, &prompt, &mut |delta| {
            let _ = tx.send(StreamEvent::Delta(delta.to_string()));
        });
        let _ = tx.send(StreamEvent::Finished(result));
    });
}

fn append_text(message: &mut Message, extra: &str) {
    if let Some(Part::Text { text }) = message.parts.last_mut() {
        text.push_str(extra);
        return;
    }
    message.parts.push(Part::text(extra));
}
