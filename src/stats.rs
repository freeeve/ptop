use std::cell::RefCell;
use std::collections::VecDeque;
use std::time::{Duration, Instant};
use tdigest::TDigest;

/// Maximum number of samples to keep in history.
const MAX_HISTORY: usize = 300;

/// Ping result for a single ping attempt.
#[derive(Debug, Clone)]
pub enum PingResult {
    Success(Duration),
    Timeout,
    #[allow(dead_code)]
    Error(String),
}

/// All-time statistics using t-digest for streaming percentiles.
pub struct AllTimeStats {
    pub min: Option<Duration>,
    pub max: Option<Duration>,
    pub sum: Duration,
    pub count: u64,
    /// T-digest and buffer wrapped in RefCell for interior mutability.
    digest_state: RefCell<DigestState>,
}

struct DigestState {
    digest: TDigest,
    buffer: Vec<f64>,
}

impl std::fmt::Debug for AllTimeStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AllTimeStats")
            .field("min", &self.min)
            .field("max", &self.max)
            .field("sum", &self.sum)
            .field("count", &self.count)
            .finish()
    }
}

const BUFFER_SIZE: usize = 10;

impl Default for AllTimeStats {
    fn default() -> Self {
        Self {
            min: None,
            max: None,
            sum: Duration::ZERO,
            count: 0,
            digest_state: RefCell::new(DigestState {
                digest: TDigest::new_with_size(100),
                buffer: Vec::with_capacity(BUFFER_SIZE),
            }),
        }
    }
}

impl AllTimeStats {
    pub fn record(&mut self, d: Duration) {
        self.min = Some(self.min.map_or(d, |m| m.min(d)));
        self.max = Some(self.max.map_or(d, |m| m.max(d)));
        self.sum += d;
        self.count += 1;

        // Buffer values and merge in batches for efficiency
        let mut state = self.digest_state.borrow_mut();
        state.buffer.push(d.as_secs_f64() * 1000.0); // Store as milliseconds
        if state.buffer.len() >= BUFFER_SIZE {
            Self::flush_buffer_inner(&mut state);
        }
    }

    fn flush_buffer_inner(state: &mut DigestState) {
        if !state.buffer.is_empty() {
            let batch_digest = state
                .digest
                .merge_unsorted(std::mem::take(&mut state.buffer));
            state.digest = batch_digest;
        }
    }

    pub fn average(&self) -> Option<Duration> {
        if self.count == 0 {
            None
        } else {
            Some(self.sum / self.count as u32)
        }
    }

    /// Returns the estimated percentile (0.0 to 1.0).
    pub fn percentile(&self, p: f64) -> Option<Duration> {
        if self.count == 0 {
            return None;
        }
        // Flush buffer to include all recent samples
        let mut state = self.digest_state.borrow_mut();
        Self::flush_buffer_inner(&mut state);
        let ms = state.digest.estimate_quantile(p);
        if ms <= 0.0 {
            return None;
        }
        Some(Duration::from_secs_f64(ms / 1000.0))
    }

    pub fn p50(&self) -> Option<Duration> {
        self.percentile(0.5)
    }

    pub fn p95(&self) -> Option<Duration> {
        self.percentile(0.95)
    }
}

/// Statistics for a single target.
#[derive(Debug)]
pub struct TargetStats {
    /// Recent ping results (for sparkline).
    history: VecDeque<PingResult>,
    /// Total pings sent.
    pub sent: u64,
    /// Total successful pings.
    pub received: u64,
    /// All-time statistics.
    pub all_time: AllTimeStats,
    /// When tracking started.
    pub started_at: Instant,
    /// Current streak of successful pings.
    pub current_streak: u64,
    /// Longest streak of successful pings.
    pub longest_streak: u64,
    /// Time of last packet loss.
    pub last_loss_at: Option<Instant>,
    /// For jitter calculation: previous successful latency.
    prev_latency: Option<Duration>,
    /// Sum of absolute differences between consecutive latencies.
    jitter_sum: Duration,
    /// Count for jitter calculation.
    jitter_count: u64,
}

impl Default for TargetStats {
    fn default() -> Self {
        Self::new()
    }
}

impl TargetStats {
    pub fn new() -> Self {
        Self {
            history: VecDeque::with_capacity(MAX_HISTORY),
            sent: 0,
            received: 0,
            all_time: AllTimeStats::default(),
            started_at: Instant::now(),
            current_streak: 0,
            longest_streak: 0,
            last_loss_at: None,
            prev_latency: None,
            jitter_sum: Duration::ZERO,
            jitter_count: 0,
        }
    }

    /// Resets all statistics (window only, all-time persists).
    #[allow(dead_code)]
    pub fn reset_window(&mut self) {
        self.history.clear();
    }

    /// Resets everything including all-time stats.
    pub fn reset(&mut self) {
        self.history.clear();
        self.sent = 0;
        self.received = 0;
        self.all_time = AllTimeStats::default();
        self.started_at = Instant::now();
        self.current_streak = 0;
        self.longest_streak = 0;
        self.last_loss_at = None;
        self.prev_latency = None;
        self.jitter_sum = Duration::ZERO;
        self.jitter_count = 0;
    }

    /// Records a ping result.
    pub fn record(&mut self, result: PingResult) {
        self.sent += 1;

        match &result {
            PingResult::Success(d) => {
                self.received += 1;
                self.all_time.record(*d);

                // Update streak
                self.current_streak += 1;
                if self.current_streak > self.longest_streak {
                    self.longest_streak = self.current_streak;
                }

                // Calculate jitter (RFC 3550 style: running mean of differences)
                if let Some(prev) = self.prev_latency {
                    let diff = d.abs_diff(prev);
                    self.jitter_sum += diff;
                    self.jitter_count += 1;
                }
                self.prev_latency = Some(*d);
            }
            PingResult::Timeout | PingResult::Error(_) => {
                self.current_streak = 0;
                self.last_loss_at = Some(Instant::now());
                self.prev_latency = None; // Reset jitter tracking on loss
            }
        }

        if self.history.len() >= MAX_HISTORY {
            self.history.pop_front();
        }
        self.history.push_back(result);
    }

    /// Returns how long stats have been tracked.
    pub fn elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }

    /// Returns average jitter (mean absolute deviation between consecutive pings).
    pub fn jitter(&self) -> Option<Duration> {
        if self.jitter_count == 0 {
            None
        } else {
            Some(self.jitter_sum / self.jitter_count as u32)
        }
    }

    /// Returns time since last packet loss.
    pub fn time_since_last_loss(&self) -> Option<Duration> {
        self.last_loss_at.map(|t| t.elapsed())
    }

    /// Calculates MOS (Mean Opinion Score) based on latency, jitter, and loss.
    /// Returns a score from 1.0 (bad) to 5.0 (excellent).
    pub fn mos_score(&self) -> Option<f64> {
        let avg_latency = self.all_time.average()?.as_secs_f64() * 1000.0; // ms
        let jitter = self.jitter().unwrap_or(Duration::ZERO).as_secs_f64() * 1000.0; // ms
        let loss_pct = self.packet_loss();

        // Simplified E-model calculation
        // R = 93.2 - latency_factor - jitter_factor - loss_factor
        let effective_latency = avg_latency + jitter * 2.0 + 10.0; // Account for codec delay

        let latency_factor = if effective_latency < 160.0 {
            effective_latency / 40.0
        } else {
            (effective_latency - 120.0) / 10.0
        };

        let loss_factor = loss_pct * 2.5; // Each % of loss reduces quality

        let r_value = (93.2 - latency_factor - loss_factor).clamp(0.0, 100.0);

        // Convert R-value to MOS
        let mos = if r_value < 0.0 {
            1.0
        } else if r_value > 100.0 {
            4.5
        } else {
            1.0 + 0.035 * r_value + r_value * (r_value - 60.0) * (100.0 - r_value) * 7e-6
        };

        Some(mos.clamp(1.0, 5.0))
    }

    /// Returns a quality grade based on MOS score.
    pub fn quality_grade(&self) -> Option<(&'static str, &'static str)> {
        let mos = self.mos_score()?;
        Some(if mos >= 4.3 {
            ("A", "Excellent")
        } else if mos >= 4.0 {
            ("B", "Good")
        } else if mos >= 3.6 {
            ("C", "Fair")
        } else if mos >= 3.1 {
            ("D", "Poor")
        } else {
            ("F", "Bad")
        })
    }

    /// Returns histogram buckets for latency distribution.
    /// Returns (bucket_boundaries_ms, counts).
    pub fn histogram(&self, num_buckets: usize) -> Option<(Vec<f64>, Vec<u64>)> {
        let latencies: Vec<f64> = self
            .history
            .iter()
            .filter_map(|r| match r {
                PingResult::Success(d) => Some(d.as_secs_f64() * 1000.0),
                _ => None,
            })
            .collect();

        if latencies.is_empty() {
            return None;
        }

        let min = latencies.iter().cloned().fold(f64::INFINITY, f64::min);
        let max = latencies.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

        if (max - min).abs() < 0.001 {
            // All values are the same
            return Some((vec![min], vec![latencies.len() as u64]));
        }

        let bucket_size = (max - min) / num_buckets as f64;
        let mut boundaries = Vec::with_capacity(num_buckets);
        let mut counts = vec![0u64; num_buckets];

        for i in 0..num_buckets {
            boundaries.push(min + bucket_size * i as f64);
        }

        for lat in &latencies {
            let bucket = ((lat - min) / bucket_size).floor() as usize;
            let bucket = bucket.min(num_buckets - 1);
            counts[bucket] += 1;
        }

        Some((boundaries, counts))
    }

    /// Returns all-time packet loss percentage.
    pub fn packet_loss(&self) -> f64 {
        if self.sent == 0 {
            return 0.0;
        }
        ((self.sent - self.received) as f64 / self.sent as f64) * 100.0
    }

    /// Returns window (recent history) packet loss stats: (lost_count, loss_percentage).
    pub fn window_packet_loss(&self) -> (u64, f64) {
        if self.history.is_empty() {
            return (0, 0.0);
        }
        let total = self.history.len() as u64;
        let successful = self
            .history
            .iter()
            .filter(|r| matches!(r, PingResult::Success(_)))
            .count() as u64;
        let lost = total - successful;
        let pct = (lost as f64 / total as f64) * 100.0;
        (lost, pct)
    }

    /// Returns all-time packet loss stats: (lost_count, loss_percentage).
    pub fn all_time_packet_loss(&self) -> (u64, f64) {
        let lost = self.sent - self.received;
        (lost, self.packet_loss())
    }

    /// Returns the most recent latency, if available.
    pub fn current(&self) -> Option<Duration> {
        self.history.back().and_then(|r| match r {
            PingResult::Success(d) => Some(*d),
            _ => None,
        })
    }

    /// Returns successful latencies from history.
    fn successful_latencies(&self) -> Vec<Duration> {
        self.history
            .iter()
            .filter_map(|r| match r {
                PingResult::Success(d) => Some(*d),
                _ => None,
            })
            .collect()
    }

    /// Returns average latency.
    pub fn average(&self) -> Option<Duration> {
        let latencies = self.successful_latencies();
        if latencies.is_empty() {
            return None;
        }
        let sum: Duration = latencies.iter().sum();
        Some(sum / latencies.len() as u32)
    }

    /// Returns minimum latency.
    pub fn min(&self) -> Option<Duration> {
        self.successful_latencies().into_iter().min()
    }

    /// Returns maximum latency.
    pub fn max(&self) -> Option<Duration> {
        self.successful_latencies().into_iter().max()
    }

    /// Returns the nth percentile latency.
    pub fn percentile(&self, p: f64) -> Option<Duration> {
        let mut latencies = self.successful_latencies();
        if latencies.is_empty() {
            return None;
        }
        latencies.sort();
        let idx = ((p / 100.0) * (latencies.len() - 1) as f64).round() as usize;
        Some(latencies[idx])
    }

    /// Returns P50 (median) latency.
    pub fn p50(&self) -> Option<Duration> {
        self.percentile(50.0)
    }

    /// Returns P95 latency.
    pub fn p95(&self) -> Option<Duration> {
        self.percentile(95.0)
    }

    /// Returns P99 latency.
    #[allow(dead_code)]
    pub fn p99(&self) -> Option<Duration> {
        self.percentile(99.0)
    }

    /// Returns the number of samples in the recent window.
    pub fn window_count(&self) -> usize {
        self.history.len()
    }

    /// Returns latencies as f64 milliseconds for sparkline rendering.
    /// Timeouts/errors are represented as 0.0.
    pub fn sparkline_data(&self) -> Vec<u64> {
        self.history
            .iter()
            .map(|r| match r {
                PingResult::Success(d) => d.as_micros() as u64,
                _ => 0,
            })
            .collect()
    }

    /// Returns the last N latencies for display.
    #[allow(dead_code)]
    pub fn recent_latencies(&self, n: usize) -> Vec<Option<Duration>> {
        self.history
            .iter()
            .rev()
            .take(n)
            .map(|r| match r {
                PingResult::Success(d) => Some(*d),
                _ => None,
            })
            .collect()
    }
}

/// Formats a duration as a human-readable string.
pub fn format_duration(d: Duration) -> String {
    let micros = d.as_micros();
    if micros < 1000 {
        format!("{}µs", micros)
    } else if micros < 100_000 {
        format!("{:.1}ms", micros as f64 / 1000.0)
    } else {
        format!("{}ms", d.as_millis())
    }
}

/// Formats an optional duration, returning "-" if None.
pub fn format_duration_opt(d: Option<Duration>) -> String {
    d.map(format_duration).unwrap_or_else(|| "-".to_string())
}

/// Formats elapsed time as human-readable (e.g., "5m 32s", "1h 5m").
pub fn format_elapsed(d: Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        let hours = secs / 3600;
        let mins = (secs % 3600) / 60;
        format!("{}h {}m", hours, mins)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_duration_microseconds() {
        assert_eq!(format_duration(Duration::from_micros(500)), "500µs");
        assert_eq!(format_duration(Duration::from_micros(999)), "999µs");
    }

    #[test]
    fn test_format_duration_milliseconds() {
        assert_eq!(format_duration(Duration::from_micros(1500)), "1.5ms");
        assert_eq!(format_duration(Duration::from_millis(15)), "15.0ms");
        assert_eq!(format_duration(Duration::from_millis(99)), "99.0ms");
    }

    #[test]
    fn test_format_duration_large() {
        assert_eq!(format_duration(Duration::from_millis(100)), "100ms");
        assert_eq!(format_duration(Duration::from_millis(1500)), "1500ms");
    }

    #[test]
    fn test_format_duration_opt() {
        assert_eq!(format_duration_opt(None), "-");
        assert_eq!(
            format_duration_opt(Some(Duration::from_millis(10))),
            "10.0ms"
        );
    }

    #[test]
    fn test_format_elapsed() {
        assert_eq!(format_elapsed(Duration::from_secs(30)), "30s");
        assert_eq!(format_elapsed(Duration::from_secs(90)), "1m 30s");
        assert_eq!(format_elapsed(Duration::from_secs(3661)), "1h 1m");
    }

    #[test]
    fn test_target_stats_new() {
        let stats = TargetStats::new();
        assert_eq!(stats.sent, 0);
        assert_eq!(stats.received, 0);
        assert_eq!(stats.current_streak, 0);
        assert_eq!(stats.longest_streak, 0);
    }

    #[test]
    fn test_target_stats_record_success() {
        let mut stats = TargetStats::new();
        stats.record(PingResult::Success(Duration::from_millis(10)));

        assert_eq!(stats.sent, 1);
        assert_eq!(stats.received, 1);
        assert_eq!(stats.current_streak, 1);
        assert_eq!(stats.packet_loss(), 0.0);
    }

    #[test]
    fn test_target_stats_record_timeout() {
        let mut stats = TargetStats::new();
        stats.record(PingResult::Success(Duration::from_millis(10)));
        stats.record(PingResult::Timeout);

        assert_eq!(stats.sent, 2);
        assert_eq!(stats.received, 1);
        assert_eq!(stats.current_streak, 0);
        assert_eq!(stats.packet_loss(), 50.0);
    }

    #[test]
    fn test_target_stats_streak_tracking() {
        let mut stats = TargetStats::new();

        // Build a streak of 5
        for _ in 0..5 {
            stats.record(PingResult::Success(Duration::from_millis(10)));
        }
        assert_eq!(stats.current_streak, 5);
        assert_eq!(stats.longest_streak, 5);

        // Break the streak
        stats.record(PingResult::Timeout);
        assert_eq!(stats.current_streak, 0);
        assert_eq!(stats.longest_streak, 5);

        // Start new streak
        for _ in 0..3 {
            stats.record(PingResult::Success(Duration::from_millis(10)));
        }
        assert_eq!(stats.current_streak, 3);
        assert_eq!(stats.longest_streak, 5); // Still 5
    }

    #[test]
    fn test_target_stats_latency_stats() {
        let mut stats = TargetStats::new();

        stats.record(PingResult::Success(Duration::from_millis(10)));
        stats.record(PingResult::Success(Duration::from_millis(20)));
        stats.record(PingResult::Success(Duration::from_millis(30)));

        assert_eq!(stats.min(), Some(Duration::from_millis(10)));
        assert_eq!(stats.max(), Some(Duration::from_millis(30)));
        assert_eq!(stats.average(), Some(Duration::from_millis(20)));
        assert_eq!(stats.current(), Some(Duration::from_millis(30)));
    }

    #[test]
    fn test_target_stats_percentiles() {
        let mut stats = TargetStats::new();

        // Add 100 samples: 1ms, 2ms, ..., 100ms
        for i in 1..=100 {
            stats.record(PingResult::Success(Duration::from_millis(i)));
        }

        let p50 = stats.p50().unwrap();
        let p95 = stats.p95().unwrap();

        // P50 should be around 50ms
        assert!(p50.as_millis() >= 45 && p50.as_millis() <= 55);
        // P95 should be around 95ms
        assert!(p95.as_millis() >= 90 && p95.as_millis() <= 100);
    }

    #[test]
    fn test_target_stats_jitter() {
        let mut stats = TargetStats::new();

        stats.record(PingResult::Success(Duration::from_millis(10)));
        stats.record(PingResult::Success(Duration::from_millis(20)));
        stats.record(PingResult::Success(Duration::from_millis(10)));

        // Jitter should be average of |20-10| and |10-20| = 10ms
        let jitter = stats.jitter().unwrap();
        assert_eq!(jitter, Duration::from_millis(10));
    }

    #[test]
    fn test_target_stats_reset() {
        let mut stats = TargetStats::new();

        stats.record(PingResult::Success(Duration::from_millis(10)));
        stats.record(PingResult::Success(Duration::from_millis(20)));

        stats.reset();

        assert_eq!(stats.sent, 0);
        assert_eq!(stats.received, 0);
        assert_eq!(stats.current_streak, 0);
        assert!(stats.current().is_none());
    }

    #[test]
    fn test_target_stats_histogram() {
        let mut stats = TargetStats::new();

        // Add samples clustered around 10ms and 20ms
        for _ in 0..10 {
            stats.record(PingResult::Success(Duration::from_millis(10)));
        }
        for _ in 0..5 {
            stats.record(PingResult::Success(Duration::from_millis(20)));
        }

        let (boundaries, counts) = stats.histogram(4).unwrap();

        assert_eq!(boundaries.len(), 4);
        assert_eq!(counts.len(), 4);
        assert_eq!(counts.iter().sum::<u64>(), 15); // Total samples
    }

    #[test]
    fn test_all_time_stats() {
        let mut stats = TargetStats::new();

        for i in 1..=20 {
            stats.record(PingResult::Success(Duration::from_millis(i)));
        }

        let all_time = &stats.all_time;
        assert_eq!(all_time.count, 20);
        assert_eq!(all_time.min, Some(Duration::from_millis(1)));
        assert_eq!(all_time.max, Some(Duration::from_millis(20)));

        let avg = all_time.average().unwrap();
        // Average of 1..=20 is 10.5
        assert!(avg.as_millis() >= 10 && avg.as_millis() <= 11);
    }

    #[test]
    fn test_mos_score_excellent() {
        let mut stats = TargetStats::new();

        // Low latency, no jitter, no loss = excellent quality
        for _ in 0..100 {
            stats.record(PingResult::Success(Duration::from_millis(10)));
        }

        let mos = stats.mos_score().unwrap();
        assert!(
            mos >= 4.0,
            "MOS should be >= 4.0 for excellent quality, got {}",
            mos
        );
    }

    #[test]
    fn test_mos_score_poor() {
        let mut stats = TargetStats::new();

        // High latency = poor quality
        for _ in 0..100 {
            stats.record(PingResult::Success(Duration::from_millis(500)));
        }

        let mos = stats.mos_score().unwrap();
        assert!(
            mos < 3.5,
            "MOS should be < 3.5 for poor quality, got {}",
            mos
        );
    }

    #[test]
    fn test_quality_grade() {
        let mut stats = TargetStats::new();

        // Good quality
        for _ in 0..100 {
            stats.record(PingResult::Success(Duration::from_millis(15)));
        }

        let (grade, _desc) = stats.quality_grade().unwrap();
        assert!(
            grade == "A" || grade == "B",
            "Expected A or B grade, got {}",
            grade
        );
    }

    #[test]
    fn test_sparkline_data() {
        let mut stats = TargetStats::new();

        stats.record(PingResult::Success(Duration::from_millis(10)));
        stats.record(PingResult::Timeout);
        stats.record(PingResult::Success(Duration::from_millis(20)));

        let data = stats.sparkline_data();

        assert_eq!(data.len(), 3);
        assert_eq!(data[0], 10_000); // 10ms in microseconds
        assert_eq!(data[1], 0); // Timeout
        assert_eq!(data[2], 20_000); // 20ms in microseconds
    }

    #[test]
    fn test_packet_loss_calculation() {
        let mut stats = TargetStats::new();

        // 8 successes, 2 failures = 20% loss
        for _ in 0..8 {
            stats.record(PingResult::Success(Duration::from_millis(10)));
        }
        stats.record(PingResult::Timeout);
        stats.record(PingResult::Timeout);

        assert!((stats.packet_loss() - 20.0).abs() < 0.01);
    }
}
