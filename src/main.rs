mod app;
mod config;
mod logging;
mod ping;
mod replay;
mod stats;
mod ui;

use anyhow::Result;
use app::App;
use clap::Parser;
use config::{Args, build_target_list};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::prelude::*;
use replay::ReplayState;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

/// Checks if we likely have permission to send ICMP packets.
fn check_icmp_permissions() -> bool {
    // On Unix, check if we're root or have CAP_NET_RAW
    #[cfg(unix)]
    {
        // Check if running as root
        if unsafe { libc::geteuid() } == 0 {
            return true;
        }

        // On Linux, check if unprivileged ICMP is enabled
        #[cfg(target_os = "linux")]
        {
            if let Ok(content) = std::fs::read_to_string("/proc/sys/net/ipv4/ping_group_range") {
                let parts: Vec<&str> = content.trim().split_whitespace().collect();
                if parts.len() == 2 {
                    if let (Ok(min), Ok(max)) = (parts[0].parse::<u32>(), parts[1].parse::<u32>()) {
                        let gid = unsafe { libc::getegid() };
                        if gid >= min && gid <= max {
                            return true;
                        }
                    }
                }
            }
        }

        false
    }

    #[cfg(not(unix))]
    true
}

/// Prints a helpful error message about ICMP permissions.
fn print_permission_help() {
    eprintln!("Error: ptop requires elevated privileges to send ICMP packets.\n");
    eprintln!("Solutions:");
    eprintln!("  1. Run with sudo:");
    eprintln!("     sudo ptop\n");

    #[cfg(target_os = "linux")]
    {
        eprintln!("  2. Set capabilities on the binary:");
        eprintln!("     sudo setcap cap_net_raw=ep $(which ptop)\n");
        eprintln!("  3. Enable unprivileged ICMP (system-wide):");
        eprintln!("     sudo sysctl net.ipv4.ping_group_range=\"0 2147483647\"");
    }

    #[cfg(target_os = "macos")]
    {
        eprintln!("  2. On macOS, sudo is typically required for ICMP.");
    }
}

/// How often to refresh the UI.
const UI_TICK_RATE: Duration = Duration::from_millis(100);

#[tokio::main]
async fn main() -> Result<()> {
    // Parse CLI arguments
    let args = Args::parse();

    // Handle --list-logs
    if args.list_logs {
        return list_available_logs();
    }

    // Handle --replay
    if let Some(replay_path) = &args.replay {
        return run_replay_mode(replay_path, args.speed).await;
    }

    // Normal live mode
    run_live_mode(args).await
}

/// Lists available log files for replay.
fn list_available_logs() -> Result<()> {
    let logs = logging::list_logs()?;

    if logs.is_empty() {
        println!("No log files found.");
        println!("Use -l flag to record logs: sudo ptop -l");
        return Ok(());
    }

    println!("Available log files for replay:\n");
    for log in logs {
        println!("  {}", log.display());
        // Try to get file size
        if let Ok(meta) = std::fs::metadata(&log) {
            let size_kb = meta.len() / 1024;
            println!("    Size: {} KB", size_kb);
        }
    }
    println!("\nUse --replay <path> to replay a log file.");

    Ok(())
}

/// Runs the application in replay mode.
async fn run_replay_mode(path: &str, speed: f64) -> Result<()> {
    let path = PathBuf::from(path);

    if !path.exists() {
        eprintln!("Log file not found: {}", path.display());
        std::process::exit(1);
    }

    // Load replay state
    let mut replay = ReplayState::new(&path, speed)?;
    let events = logging::load_events(&path)?;
    let (targets, mut stats) = replay::build_replay_targets(&events);

    if targets.is_empty() {
        eprintln!("No valid targets found in log file.");
        std::process::exit(1);
    }

    println!(
        "Replaying {} events at {}x speed...",
        replay.total_events(),
        speed
    );

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Main replay loop
    let res = run_replay_app(&mut terminal, &mut replay, &targets, &mut stats).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(e) = res {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }

    println!("Replay finished.");
    Ok(())
}

/// Runs the application in live mode.
async fn run_live_mode(args: Args) -> Result<()> {
    // Check permissions before starting
    if !check_icmp_permissions() {
        print_permission_help();
        std::process::exit(1);
    }

    let targets = build_target_list(&args);

    if targets.is_empty() {
        eprintln!("No targets specified. Use -t to add targets or -d to include defaults.");
        std::process::exit(1);
    }

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app
    let mut app = App::new(targets, Duration::from_millis(args.interval), args.log_raw)?;

    if args.log_raw
        && let Some(path) = &app.logger.event_log_path
    {
        eprintln!("Logging to: {}", path.display());
    }

    // Main loop
    let res = run_live_app(&mut terminal, &mut app).await;

    // Write session summary before restoring terminal
    let summary_path = app.logger.write_summary(&app.targets, &app.stats)?;
    app.logger.finish()?;

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    // Print summary location
    println!("Session summary saved to: {}", summary_path.display());
    if let Some(log_path) = &app.logger.event_log_path {
        println!("Raw ping log saved to: {}", log_path.display());
    }

    if let Err(e) = res {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }

    Ok(())
}

/// Main application loop for live mode.
async fn run_live_app<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> Result<()> {
    loop {
        app.process_updates();
        terminal.draw(|f| ui::render(f, app))?;

        if event::poll(UI_TICK_RATE)?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            use app::ViewMode;
            match app.view_mode {
                ViewMode::List => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => app.quit(),
                    KeyCode::Up | KeyCode::Char('k') => app.select_previous(),
                    KeyCode::Down | KeyCode::Char('j') => app.select_next(),
                    KeyCode::Char('r') => app.reset_stats(),
                    KeyCode::Enter => app.show_detail(),
                    _ => {}
                },
                ViewMode::Detail => match key.code {
                    KeyCode::Char('q') => app.quit(),
                    KeyCode::Esc | KeyCode::Backspace => app.show_list(),
                    KeyCode::Up | KeyCode::Char('k') => app.select_previous(),
                    KeyCode::Down | KeyCode::Char('j') => app.select_next(),
                    KeyCode::Char('r') => app.reset_stats(),
                    _ => {}
                },
            }
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

/// Main application loop for replay mode.
async fn run_replay_app<B: Backend>(
    terminal: &mut Terminal<B>,
    replay: &mut ReplayState,
    targets: &[config::Target],
    stats: &mut [stats::TargetStats],
) -> Result<()> {
    let mut selected: usize = 0;
    let mut should_quit = false;

    loop {
        // Process replay events
        let events = replay.poll_events();
        for event in events {
            replay::apply_event(event, targets, stats);
        }

        // Draw UI
        terminal.draw(|f| ui::render_replay(f, targets, stats, replay, selected))?;

        // Handle input
        if event::poll(UI_TICK_RATE)?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => should_quit = true,
                KeyCode::Char(' ') => replay.toggle_pause(),
                KeyCode::Up | KeyCode::Char('k') => {
                    selected = selected.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if selected < targets.len().saturating_sub(1) {
                        selected += 1;
                    }
                }
                KeyCode::Right | KeyCode::Char('l') => replay.skip_forward(100),
                KeyCode::Left | KeyCode::Char('h') => replay.skip_backward(100),
                KeyCode::Char('+') | KeyCode::Char('=') => replay.speed_up(),
                KeyCode::Char('-') => replay.slow_down(),
                KeyCode::Char('r') => {
                    // Reset stats
                    for stat in stats.iter_mut() {
                        stat.reset();
                    }
                }
                _ => {}
            }
        }

        if should_quit {
            break;
        }

        // Auto-quit when replay finishes (optional - could also pause)
        if replay.finished {
            // Keep running so user can review final state
        }
    }

    Ok(())
}
