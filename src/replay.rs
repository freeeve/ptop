use crate::config::Target;
use crate::logging::{PingEvent, load_events};
use crate::stats::{PingResult, TargetStats};
use anyhow::Result;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

/// Replay state for playing back recorded sessions.
pub struct ReplayState {
    /// All events loaded from the log.
    events: Vec<PingEvent>,
    /// Current position in the event stream.
    current_idx: usize,
    /// When replay started (wall clock).
    replay_started: std::time::Instant,
    /// Timestamp of first event in the log.
    log_start_time: DateTime<Utc>,
    /// Speed multiplier.
    speed: f64,
    /// Whether replay is paused.
    pub paused: bool,
    /// Whether replay has finished.
    pub finished: bool,
}

impl ReplayState {
    /// Loads events from a log file and prepares for replay.
    pub fn new(path: &PathBuf, speed: f64) -> Result<Self> {
        let events = load_events(path)?;

        if events.is_empty() {
            anyhow::bail!("Log file is empty");
        }

        let log_start_time = events[0].timestamp;

        Ok(Self {
            events,
            current_idx: 0,
            replay_started: std::time::Instant::now(),
            log_start_time,
            speed: speed.max(0.1), // Minimum 0.1x speed
            paused: false,
            finished: false,
        })
    }

    /// Returns the number of events in the replay.
    pub fn total_events(&self) -> usize {
        self.events.len()
    }

    /// Returns the current event index.
    pub fn current_event(&self) -> usize {
        self.current_idx
    }

    /// Returns the replay progress as a percentage.
    pub fn progress(&self) -> f64 {
        if self.events.is_empty() {
            return 100.0;
        }
        (self.current_idx as f64 / self.events.len() as f64) * 100.0
    }

    /// Returns the timestamp of the current position in the original log.
    pub fn current_log_time(&self) -> Option<DateTime<Utc>> {
        self.events.get(self.current_idx).map(|e| e.timestamp)
    }

    /// Returns the original log duration.
    #[allow(dead_code)]
    pub fn log_duration(&self) -> chrono::Duration {
        if let (Some(first), Some(last)) = (self.events.first(), self.events.last()) {
            last.timestamp.signed_duration_since(first.timestamp)
        } else {
            chrono::Duration::zero()
        }
    }

    /// Toggles pause state.
    pub fn toggle_pause(&mut self) {
        self.paused = !self.paused;
        if !self.paused {
            // Reset replay start time to account for pause
            self.replay_started = std::time::Instant::now();
            // Adjust log start time to current position
            if let Some(event) = self.events.get(self.current_idx) {
                self.log_start_time = event.timestamp;
            }
        }
    }

    /// Processes events that should have occurred by now.
    /// Returns events to be applied to stats.
    pub fn poll_events(&mut self) -> Vec<&PingEvent> {
        if self.paused || self.finished {
            return Vec::new();
        }

        let elapsed = self.replay_started.elapsed();
        let scaled_elapsed = Duration::from_secs_f64(elapsed.as_secs_f64() * self.speed);
        let current_replay_time =
            self.log_start_time + chrono::Duration::from_std(scaled_elapsed).unwrap_or_default();

        let mut ready_events = Vec::new();

        while self.current_idx < self.events.len() {
            let event = &self.events[self.current_idx];
            if event.timestamp <= current_replay_time {
                ready_events.push(event);
                self.current_idx += 1;
            } else {
                break;
            }
        }

        if self.current_idx >= self.events.len() {
            self.finished = true;
        }

        ready_events
    }

    /// Skips forward by a number of events.
    pub fn skip_forward(&mut self, count: usize) {
        self.current_idx = (self.current_idx + count).min(self.events.len().saturating_sub(1));
        self.replay_started = std::time::Instant::now();
        if let Some(event) = self.events.get(self.current_idx) {
            self.log_start_time = event.timestamp;
        }
    }

    /// Skips backward by a number of events.
    pub fn skip_backward(&mut self, count: usize) {
        self.current_idx = self.current_idx.saturating_sub(count);
        self.finished = false;
        self.replay_started = std::time::Instant::now();
        if let Some(event) = self.events.get(self.current_idx) {
            self.log_start_time = event.timestamp;
        }
    }

    /// Increases replay speed.
    pub fn speed_up(&mut self) {
        self.speed = (self.speed * 2.0).min(100.0);
    }

    /// Decreases replay speed.
    pub fn slow_down(&mut self) {
        self.speed = (self.speed / 2.0).max(0.1);
    }

    /// Returns current speed.
    pub fn speed(&self) -> f64 {
        self.speed
    }
}

/// Builds targets and initial stats from replay events.
pub fn build_replay_targets(events: &[PingEvent]) -> (Vec<Target>, Vec<TargetStats>) {
    let mut target_map: HashMap<(String, String), usize> = HashMap::new();
    let mut targets = Vec::new();
    let mut stats = Vec::new();

    for event in events {
        let key = (event.target_name.clone(), event.target_addr.clone());
        if let std::collections::hash_map::Entry::Vacant(e) = target_map.entry(key) {
            let idx = targets.len();
            e.insert(idx);

            if let Ok(addr) = event.target_addr.parse() {
                targets.push(Target::new(event.target_name.clone(), addr));
                stats.push(TargetStats::new());
            }
        }
    }

    (targets, stats)
}

/// Applies a replay event to the appropriate stats.
pub fn apply_event(event: &PingEvent, targets: &[Target], stats: &mut [TargetStats]) {
    // Find the target by address
    for (idx, target) in targets.iter().enumerate() {
        if target.addr.to_string() == event.target_addr {
            let result = match event.latency_us {
                Some(us) => PingResult::Success(Duration::from_micros(us)),
                None => PingResult::Timeout,
            };
            stats[idx].record(result);
            break;
        }
    }
}
