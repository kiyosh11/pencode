use anyhow::Context;
use clap::{Parser, Subcommand};

/// pencode — the open source coding agent, in Rust.
#[derive(Parser)]
#[command(name = "pencode", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run a one-shot prompt against the agent.
    Run {
        /// The prompt text; reads from stdin if omitted.
        prompt: Option<String>,
        /// Directory to operate in (defaults to cwd).
        #[arg(long)]
        directory: Option<String>,
    },
    /// Start the HTTP API server.
    Serve {
        #[arg(long, default_value_t = 4096)]
        port: u16,
    },
    /// Launch the interactive terminal UI.
    Tui,
    /// List available models.
    Models,
    /// Show authentication status for configured providers.
    Auth,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let runtime = tokio::runtime::Runtime::new()?;
    
    match cli.command {
        Command::Run { prompt, directory } => run_prompt(prompt, directory),
        Command::Serve { port } => runtime.block_on(serve(port)),
        Command::Tui => tui(),
        Command::Models => models(),
        Command::Auth => auth(),
    }
}

fn load_app(directory: Option<String>) -> anyhow::Result<pencode_core::App> {
    let directory = match directory {
        Some(dir) => std::path::PathBuf::from(dir),
        None => std::env::current_dir().context("resolving cwd")?,
    };
    pencode_core::App::load(directory)
}

fn run_prompt(prompt: Option<String>, directory: Option<String>) -> anyhow::Result<()> {
    let app = load_app(directory)?;
    let prompt = match prompt {
        Some(text) => text,
        None => {
            let mut buffer = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut buffer)?;
            buffer
        }
    };

    let store = app.store();
    let sessions = store.list()?;
    let session = match sessions.into_iter().next() {
        Some(existing) => existing,
        None => store.create(".")?,
    };

    use pencode_protocol::{Message, Part, Role};
    let user_message = Message::new(Role::User, vec![Part::text(&prompt)]);
    let session = store.append(&session.id, user_message)?;

    println!("session {}", session.id);
    println!("model  {}", app.config().model.clone().unwrap_or_else(|| "anthropic/claude-sonnet-4-5 (default)".into()));

    let model_spec = app
        .config()
        .model
        .clone()
        .unwrap_or_else(|| "anthropic/claude-sonnet-4-5".to_string());
    match pencode_provider::resolve(&model_spec, app.config()) {
        Ok(resolved) => {
            print!("pencode ");
            use std::io::Write;
            std::io::stdout().flush()?;
            let reply = pencode_provider::stream(
                &resolved,
                &pencode_provider::Prompt::from_session(&session),
                &mut |delta| {
                    print!("{delta}");
                    let _ = std::io::stdout().flush();
                },
            );
            match reply {
                Ok(full) => {
                    let reply_message = Message::new(Role::Assistant, vec![Part::text(full)]);
                    store.append(&session.id, reply_message)?;
                    println!();
                }
                Err(err) => {
                    println!("\n⚠ {err:#}");
                }
            }
        }
        Err(err) => {
            println!("⚠ prompt stored, but no provider available: {err:#}");
        }
    }
    Ok(())
}

async fn serve(port: u16) -> anyhow::Result<()> {
    let app = load_app(None)?;
    pencode_server::serve(app, port).await
}

fn tui() -> anyhow::Result<()> {
    let app = load_app(None)?;
    pencode_tui::run(app)
}

fn models() -> anyhow::Result<()> {
    println!("configured model: {:?}", load_app(None)?.config().model);
    println!(
        "providers: {:?}",
        load_app(None)?.config().provider.keys().collect::<Vec<_>>()
    );
    Ok(())
}

fn auth() -> anyhow::Result<()> {
    let config = load_app(None)?.config().clone();
    if config.provider.is_empty() {
        println!("no providers configured. add ~/.config/pencode/config.json");
        return Ok(());
    }
    for (name, provider) in &config.provider {
        let status = match provider.api_key.as_deref() {
            Some(_) => "authenticated",
            None => "missing api key",
        };
        println!("{name}: {status}");
    }
    Ok(())
}
