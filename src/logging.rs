use crate::config::Target;
use crate::stats::TargetStats;
use anyhow::Result;
use chrono::{DateTime, Utc};
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

/// Directory for storing ptop data.
fn data_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let dir = PathBuf::from(home).join(".ptop");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// A single ping event for logging/replay.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PingEvent {
    /// Timestamp of the ping.
    pub timestamp: DateTime<Utc>,
    /// Index of the target.
    pub target_idx: usize,
    /// Target name.
    pub target_name: String,
    /// Target address.
    pub target_addr: String,
    /// Latency in microseconds, or None for timeout/error.
    pub latency_us: Option<u64>,
}

/// Session summary for JSON export.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct SessionSummary {
    pub started: DateTime<Utc>,
    pub ended: DateTime<Utc>,
    pub duration_secs: u64,
    pub targets: Vec<TargetSummary>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct TargetSummary {
    pub name: String,
    pub addr: String,
    pub sent: u64,
    pub received: u64,
    pub loss_pct: f64,
    pub latency_ms: LatencySummary,
    pub jitter_ms: Option<f64>,
    pub mos: Option<f64>,
    pub quality_grade: Option<String>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct LatencySummary {
    pub min: Option<f64>,
    pub avg: Option<f64>,
    pub p50: Option<f64>,
    pub p95: Option<f64>,
    pub max: Option<f64>,
}

/// How often to flush logs (in number of events).
const FLUSH_INTERVAL: u64 = 50;

/// How often to save periodic summary (seconds).
const SUMMARY_INTERVAL_SECS: i64 = 60;

/// Logger for recording ping events.
pub struct SessionLogger {
    /// When the session started.
    pub started: DateTime<Utc>,
    /// Gzip encoder for writing ping events (JSONL format).
    event_writer: Option<GzEncoder<BufWriter<File>>>,
    /// Path to the event log.
    pub event_log_path: Option<PathBuf>,
    /// Event counter for periodic flushing.
    event_count: u64,
    /// When the last summary was written.
    last_summary_at: DateTime<Utc>,
    /// Path for the running summary.
    summary_path: Option<PathBuf>,
}

impl SessionLogger {
    /// Creates a new session logger.
    pub fn new(log_raw: bool, log_summary: bool) -> Result<Self> {
        let started = Utc::now();
        let (event_writer, event_log_path) = if log_raw {
            let dir = data_dir()?.join("logs");
            fs::create_dir_all(&dir)?;
            let filename = format!("{}.jsonl.gz", started.format("%Y-%m-%dT%H-%M-%S"));
            let path = dir.join(filename);

            let mut opts = OpenOptions::new();
            opts.create(true).write(true).truncate(true);
            #[cfg(unix)]
            opts.mode(0o600); // Owner read/write only

            let file = opts.open(&path)?;
            let encoder = GzEncoder::new(BufWriter::new(file), Compression::default());
            (Some(encoder), Some(path))
        } else {
            (None, None)
        };

        // Pre-create summary path only if summary logging is enabled
        let summary_path = if log_summary {
            data_dir().ok().map(|d| {
                let dir = d.join("sessions");
                let _ = fs::create_dir_all(&dir);
                let filename = format!("{}.json.gz", started.format("%Y-%m-%dT%H-%M-%S"));
                dir.join(filename)
            })
        } else {
            None
        };

        Ok(Self {
            started,
            event_writer,
            event_log_path,
            event_count: 0,
            last_summary_at: started,
            summary_path,
        })
    }

    /// Logs a ping event.
    pub fn log_ping(
        &mut self,
        target_idx: usize,
        target: &Target,
        latency: Option<Duration>,
    ) -> Result<()> {
        if let Some(writer) = &mut self.event_writer {
            let event = PingEvent {
                timestamp: Utc::now(),
                target_idx,
                target_name: target.name.clone(),
                target_addr: target.addr.to_string(),
                latency_us: latency.map(|d| d.as_micros() as u64),
            };
            let line = serde_json::to_string(&event)?;
            writeln!(writer, "{}", line)?;

            self.event_count += 1;
            if self.event_count.is_multiple_of(FLUSH_INTERVAL) {
                writer.flush()?;
            }
        }
        Ok(())
    }

    /// Flushes the event log without closing it.
    #[allow(dead_code)]
    pub fn flush(&mut self) -> Result<()> {
        if let Some(writer) = &mut self.event_writer {
            writer.flush()?;
        }
        Ok(())
    }

    /// Finishes writing and closes the log file.
    pub fn finish(&mut self) -> Result<()> {
        if let Some(writer) = self.event_writer.take() {
            writer.finish()?;
        }
        Ok(())
    }

    /// Checks if it's time to write a periodic summary and writes it if so.
    /// Returns true if a summary was written.
    pub fn maybe_write_periodic_summary(
        &mut self,
        targets: &[Target],
        stats: &[TargetStats],
    ) -> Result<bool> {
        let now = Utc::now();
        let elapsed = now
            .signed_duration_since(self.last_summary_at)
            .num_seconds();

        if elapsed >= SUMMARY_INTERVAL_SECS {
            self.write_summary_internal(targets, stats, now)?;
            self.last_summary_at = now;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Internal summary writing logic.
    fn write_summary_internal(
        &self,
        targets: &[Target],
        stats: &[TargetStats],
        ended: DateTime<Utc>,
    ) -> Result<()> {
        let path = match &self.summary_path {
            Some(p) => p,
            None => return Ok(()),
        };

        let duration = ended.signed_duration_since(self.started);

        let target_summaries: Vec<TargetSummary> = targets
            .iter()
            .zip(stats.iter())
            .map(|(target, stat)| TargetSummary {
                name: target.name.clone(),
                addr: target.addr.to_string(),
                sent: stat.sent,
                received: stat.received,
                loss_pct: stat.packet_loss(),
                latency_ms: LatencySummary {
                    min: stat.all_time.min.map(|d| d.as_secs_f64() * 1000.0),
                    avg: stat.all_time.average().map(|d| d.as_secs_f64() * 1000.0),
                    p50: stat.all_time.p50().map(|d| d.as_secs_f64() * 1000.0),
                    p95: stat.all_time.p95().map(|d| d.as_secs_f64() * 1000.0),
                    max: stat.all_time.max.map(|d| d.as_secs_f64() * 1000.0),
                },
                jitter_ms: stat.jitter().map(|d| d.as_secs_f64() * 1000.0),
                mos: stat.mos_score(),
                quality_grade: stat.quality_grade().map(|(g, _)| g.to_string()),
            })
            .collect();

        let summary = SessionSummary {
            started: self.started,
            ended,
            duration_secs: duration.num_seconds() as u64,
            targets: target_summaries,
        };

        let mut opts = OpenOptions::new();
        opts.create(true).write(true).truncate(true);
        #[cfg(unix)]
        opts.mode(0o600); // Owner read/write only

        let file = opts.open(path)?;
        let encoder = GzEncoder::new(file, Compression::default());
        serde_json::to_writer_pretty(encoder, &summary)?;

        Ok(())
    }

    /// Writes the final session summary on exit.
    pub fn write_summary(
        &self,
        targets: &[Target],
        stats: &[TargetStats],
    ) -> Result<Option<PathBuf>> {
        if self.summary_path.is_some() {
            let ended = Utc::now();
            self.write_summary_internal(targets, stats, ended)?;
        }
        Ok(self.summary_path.clone())
    }
}

/// Maximum events to load for replay (prevents memory exhaustion).
const MAX_REPLAY_EVENTS: usize = 1_000_000;

/// Loads ping events from a gzipped JSONL log file for replay.
/// Limited to MAX_REPLAY_EVENTS to prevent memory exhaustion.
pub fn load_events(path: &PathBuf) -> Result<Vec<PingEvent>> {
    let file = File::open(path)?;
    let decoder = GzDecoder::new(file);
    let reader = BufReader::new(decoder);
    let mut events = Vec::new();

    for line in reader.lines() {
        if events.len() >= MAX_REPLAY_EVENTS {
            tracing::warn!(
                "Log file truncated at {} events to prevent memory exhaustion",
                MAX_REPLAY_EVENTS
            );
            break;
        }

        let line = line?;
        if !line.trim().is_empty() {
            let event: PingEvent = serde_json::from_str(&line)?;
            events.push(event);
        }
    }

    Ok(events)
}

/// Loads a session summary from a gzipped JSON file.
#[allow(dead_code)]
pub fn load_session(path: &PathBuf) -> Result<SessionSummary> {
    let file = File::open(path)?;
    let decoder = GzDecoder::new(file);
    let reader = BufReader::new(decoder);
    let summary: SessionSummary = serde_json::from_reader(reader)?;
    Ok(summary)
}

/// Lists available session summaries.
pub fn list_sessions() -> Result<Vec<PathBuf>> {
    let dir = data_dir()?.join("sessions");
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut sessions: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with(".json.gz"))
        .collect();

    sessions.sort();
    sessions.reverse(); // Most recent first
    Ok(sessions)
}

/// Lists available log files for replay.
pub fn list_logs() -> Result<Vec<PathBuf>> {
    let dir = data_dir()?.join("logs");
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut logs: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with(".jsonl.gz"))
        .collect();

    logs.sort();
    logs.reverse(); // Most recent first
    Ok(logs)
}
