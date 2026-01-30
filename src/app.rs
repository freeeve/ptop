use crate::config::Target;
use crate::logging::SessionLogger;
use crate::ping::{PingUpdate, spawn_pinger};
use crate::stats::{PingResult, TargetStats};
use chrono::{DateTime, Utc};
use std::time::Duration;
use tokio::sync::mpsc;

/// View mode for the application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    /// Main list view showing all targets.
    List,
    /// Detail view for a single target.
    Detail,
}

/// Main application state.
pub struct App {
    /// List of targets being pinged.
    pub targets: Vec<Target>,
    /// Statistics for each target.
    pub stats: Vec<TargetStats>,
    /// Currently selected row (for future keyboard nav).
    pub selected: usize,
    /// Whether the app should quit.
    pub should_quit: bool,
    /// Current view mode.
    pub view_mode: ViewMode,
    /// Channel receiver for ping updates.
    rx: mpsc::UnboundedReceiver<PingUpdate>,
    /// Session logger.
    pub logger: SessionLogger,
    /// Session start time.
    pub started_at: DateTime<Utc>,
}

impl App {
    /// Creates a new App and starts pinging all targets.
    pub fn new(targets: Vec<Target>, interval: Duration, log_raw: bool) -> anyhow::Result<Self> {
        let (tx, rx) = mpsc::unbounded_channel();

        let stats: Vec<TargetStats> = targets.iter().map(|_| TargetStats::new()).collect();

        // Spawn a pinger for each target
        for (idx, target) in targets.iter().enumerate() {
            spawn_pinger(idx, target.clone(), interval, tx.clone());
        }

        let logger = SessionLogger::new(log_raw)?;
        let started_at = logger.started;

        Ok(Self {
            targets,
            stats,
            selected: 0,
            should_quit: false,
            view_mode: ViewMode::List,
            rx,
            logger,
            started_at,
        })
    }

    /// Processes any pending ping updates.
    pub fn process_updates(&mut self) {
        while let Ok(update) = self.rx.try_recv() {
            if update.target_idx < self.stats.len() {
                // Log the ping event
                let latency = match &update.result {
                    PingResult::Success(d) => Some(*d),
                    _ => None,
                };
                let _ = self.logger.log_ping(
                    update.target_idx,
                    &self.targets[update.target_idx],
                    latency,
                );

                self.stats[update.target_idx].record(update.result);
            }
        }

        // Periodic summary save (every ~60s)
        let _ = self
            .logger
            .maybe_write_periodic_summary(&self.targets, &self.stats);
    }

    /// Returns session elapsed time.
    pub fn session_elapsed(&self) -> chrono::Duration {
        Utc::now().signed_duration_since(self.started_at)
    }

    /// Moves selection up.
    pub fn select_previous(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    /// Moves selection down.
    pub fn select_next(&mut self) {
        if self.selected < self.targets.len().saturating_sub(1) {
            self.selected += 1;
        }
    }

    /// Signals the app to quit.
    pub fn quit(&mut self) {
        self.should_quit = true;
    }

    /// Resets all statistics.
    pub fn reset_stats(&mut self) {
        for stat in &mut self.stats {
            stat.reset();
        }
    }

    /// Toggles to detail view for the selected target.
    pub fn show_detail(&mut self) {
        if !self.targets.is_empty() {
            self.view_mode = ViewMode::Detail;
        }
    }

    /// Returns to list view.
    pub fn show_list(&mut self) {
        self.view_mode = ViewMode::List;
    }

    /// Returns the currently selected target and its stats.
    pub fn selected_target(&self) -> Option<(&Target, &TargetStats)> {
        if self.selected < self.targets.len() {
            Some((&self.targets[self.selected], &self.stats[self.selected]))
        } else {
            None
        }
    }
}
