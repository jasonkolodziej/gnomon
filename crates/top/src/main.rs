//! `gntop`: live terminal Gantt chart of the Gnomon cluster plan.
//!
//! Interactive mode takes over the terminal (alternate screen, raw mode;
//! restored on exit and on panic). `--once` prints one plain-text frame to
//! stdout and exits non-zero when the controller was unreachable.

use clap::Parser;
use crossterm::event::{self, Event};
use gnomon_api::pb::scheduler_client::SchedulerClient;
use gnomon_top::app::{App, Key, Update};
use gnomon_top::poll::{self, default_addr, now_secs};
use gnomon_top::render;
use gnomon_top::view::{parse_duration_ms, parse_duration_secs, Window};
use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

#[derive(Parser, Debug)]
#[command(name = "gntop", about = "Live Gantt chart of the Gnomon cluster plan")]
struct Args {
    /// Controller address (default: $GNOMON_ADDR or http://127.0.0.1:7631).
    #[arg(long)]
    addr: Option<String>,
    /// Poll interval (e.g. 1s, 500ms, 5s).
    #[arg(long, default_value = "1s", value_parser = parse_duration_ms)]
    interval: u64,
    /// Time shown before now (e.g. 10m).
    #[arg(long, default_value = "10m", value_parser = parse_duration_secs)]
    history: u64,
    /// Time shown after now (e.g. 2h).
    #[arg(long, default_value = "2h", value_parser = parse_duration_secs)]
    horizon: u64,
    /// Only show jobs of this user (same as `/` in the UI).
    #[arg(long, short)]
    user: Option<String>,
    /// Render one frame to stdout as plain text and exit.
    #[arg(long)]
    once: bool,
    /// Frame width for --once (default: terminal width, else 120).
    #[arg(long)]
    width: Option<u16>,
    /// Frame height for --once (default: terminal height, else 40).
    #[arg(long)]
    height: Option<u16>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let addr = args.addr.clone().unwrap_or_else(default_addr);
    let mut app = App::new(&addr, Window::new(args.history, args.horizon), now_secs());
    app.filter = args.user.clone().unwrap_or_default();
    let interval = Duration::from_millis(args.interval.max(100));
    if args.once {
        return run_once(app, &args, interval).await;
    }
    run_interactive(app, interval).await
}

async fn run_once(mut app: App, args: &Args, interval: Duration) -> anyhow::Result<()> {
    let timeout = interval.clamp(Duration::from_secs(1), Duration::from_secs(5));
    let mut client: Option<SchedulerClient<_>> = None;
    let update = poll::poll_once(&app.addr, &mut client, timeout).await;
    let failed = matches!(update, Update::Error(_));
    app.apply(update);
    app.now = now_secs();
    let (tw, th) = if std::io::stdout().is_terminal() {
        crossterm::terminal::size().unwrap_or((120, 40))
    } else {
        (120, 40)
    };
    let frame = render::render_text(
        &app,
        args.width.unwrap_or(tw).max(20),
        args.height.unwrap_or(th).max(8),
    );
    print!("{frame}");
    if failed {
        eprintln!("gntop: controller at {} unreachable", app.addr);
        std::process::exit(1);
    }
    Ok(())
}

async fn run_interactive(mut app: App, interval: Duration) -> anyhow::Result<()> {
    let paused = Arc::new(AtomicBool::new(false));
    let (tx, mut rx) = watch::channel(None);
    let poller = tokio::spawn(poll::run_poller(
        app.addr.clone(),
        interval,
        paused.clone(),
        tx,
    ));
    // try_init installs a panic hook that restores the terminal first.
    let mut terminal = ratatui::try_init()?;
    let result = ui_loop(&mut terminal, &mut app, &paused, &mut rx);
    ratatui::try_restore()?;
    poller.abort();
    result
}

fn ui_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    paused: &AtomicBool,
    rx: &mut watch::Receiver<Option<Update>>,
) -> anyhow::Result<()> {
    while !app.quit {
        if rx.has_changed().unwrap_or(false) {
            if let Some(update) = rx.borrow_and_update().clone() {
                app.apply(update);
            }
        }
        app.now = now_secs();
        terminal.draw(|f| render::render(app, f.area(), f.buffer_mut()))?;
        // Resize events need no handling: the next draw picks up the size.
        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(k) = event::read()? {
                app.on_key(Key::from_event(k));
                paused.store(app.paused, Ordering::Relaxed);
            }
        }
    }
    Ok(())
}
