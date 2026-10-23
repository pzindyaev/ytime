mod api;
mod app;
mod calendar;
mod config;
mod input;
mod store;
mod timer;
mod ui;

use clap::{Parser, Subcommand};
use crossterm::event::{Event as CrosstermEvent, KeyEventKind};

use app::{App, AppEvent};
use config::Config;
use store::PersistedState;

#[derive(Parser)]
#[command(
    name = "ytime",
    version,
    about = "Terminal time tracking for Yandex Tracker"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Authorize by pasting a personal OAuth token (no TUI).
    Auth {
        /// Token value; skips the link/stdin prompt (for scripts).
        #[arg(long)]
        token: Option<String>,
    },
    /// Delete the stored OAuth token.
    Logout,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let cfg = Config::load()?;
    let mut state = PersistedState::load();

    match cli.cmd {
        Some(Cmd::Auth { token }) => {
            let token = match token {
                Some(t) => t,
                None => {
                    let client_id = cfg
                        .client_id
                        .clone()
                        .filter(|id| !id.is_empty())
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "client_id is not set.\n\
                                 Register an app at https://oauth.yandex.ru/client/new \
                                 (Callback URI: https://oauth.yandex.ru/verification_code, \
                                 scopes: tracker:read, tracker:write) and put the id into {}",
                                Config::path().display()
                            )
                        })?;
                    println!(
                        "Open this link and allow access:\n\
                         {}/authorize?response_type=token&client_id={client_id}",
                        cfg.oauth_base
                    );
                    print!("Paste the token: ");
                    use std::io::Write as _;
                    std::io::stdout().flush()?;
                    let mut buf = String::new();
                    std::io::stdin().read_line(&mut buf)?;
                    let t = buf.trim().to_string();
                    if t.is_empty() {
                        anyhow::bail!("empty token");
                    }
                    t
                }
            };
            state.tokens = Some(store::Tokens {
                access_token: token,
            });
            state.save()?;
            println!(
                "Token saved to {} — run `ytime` to start tracking.",
                config::state_path().display()
            );
            Ok(())
        }
        Some(Cmd::Logout) => {
            state.tokens = None;
            state.save()?;
            println!("Token removed.");
            Ok(())
        }
        None => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(async move { run_tui(cfg, state).await })
        }
    }
}

async fn run_tui(cfg: Config, state: PersistedState) -> anyhow::Result<()> {
    let (tx, rx) = tokio::sync::mpsc::channel::<AppEvent>(256);

    // keyboard: a plain OS thread feeds crossterm events into the loop
    {
        let tx = tx.clone();
        std::thread::spawn(move || loop {
            match crossterm::event::read() {
                Ok(CrosstermEvent::Key(k)) => {
                    if k.kind == KeyEventKind::Press && tx.blocking_send(AppEvent::Key(k)).is_err()
                    {
                        return;
                    }
                }
                Ok(_) => {}
                Err(_) => return,
            }
        });
    }
    // 1s heartbeat for the timer display and debounces
    {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
            loop {
                interval.tick().await;
                if tx.send(AppEvent::Tick).await.is_err() {
                    return;
                }
            }
        });
    }

    let terminal = ratatui::init();
    let result = App::new(cfg, state, tx).run(rx, terminal).await;
    ratatui::restore();
    result
}
