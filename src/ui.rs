use crate::app::{App, ViewMode};
use crate::config::Target;
use crate::replay::ReplayState;
use crate::stats::{TargetStats, format_duration_opt, format_elapsed};
use chrono::Local;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Gauge, Paragraph, Row, Sparkline, Table},
};

/// Renders the entire UI.
pub fn render(frame: &mut Frame, app: &App) {
    match app.view_mode {
        ViewMode::List => render_list_view(frame, app),
        ViewMode::Detail => render_detail_view(frame, app),
    }
}

/// Renders the list view (main view).
fn render_list_view(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(10),   // Main table
            Constraint::Length(3), // Footer/help
        ])
        .split(frame.area());

    render_header(frame, chunks[0], None, app);
    render_table(frame, chunks[1], app);
    render_footer(frame, chunks[2], ViewMode::List);
}

/// Formats session duration for display.
fn format_session_duration(duration: chrono::Duration) -> String {
    let secs = duration.num_seconds();
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

/// Formats large numbers compactly (e.g., 200000 → "200k", 3123423 → "3.1m").
fn format_count(n: u64) -> String {
    if n >= 1_000_000 {
        let m = n as f64 / 1_000_000.0;
        if m >= 10.0 {
            format!("{}m", m.round() as u64)
        } else {
            format!("{:.1}m", m)
        }
    } else if n >= 1_000 {
        let k = n as f64 / 1_000.0;
        if k >= 10.0 {
            format!("{}k", k.round() as u64)
        } else {
            format!("{:.1}k", k)
        }
    } else {
        format!("{}", n)
    }
}

/// Renders the header with title.
fn render_header(frame: &mut Frame, area: Rect, subtitle: Option<&str>, app: &App) {
    let now = Local::now();
    let session_duration = format_session_duration(app.session_elapsed());

    let mut spans = vec![
        Span::styled(
            "ptop",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" │ "),
        Span::styled(
            now.format("%H:%M:%S").to_string(),
            Style::default().fg(Color::White),
        ),
        Span::raw(" │ "),
        Span::styled(
            format!("session: {}", session_duration),
            Style::default().fg(Color::DarkGray),
        ),
    ];

    if let Some(sub) = subtitle {
        spans.push(Span::raw(" │ "));
        spans.push(Span::styled(sub, Style::default().fg(Color::Yellow)));
    }

    // Show logging indicator if enabled
    if app.logger.event_log_path.is_some() {
        spans.push(Span::raw(" │ "));
        spans.push(Span::styled("●REC", Style::default().fg(Color::Red)));
    }

    let header = Paragraph::new(Line::from(spans)).block(Block::default().borders(Borders::BOTTOM));

    frame.render_widget(header, area);
}

/// Renders the main target table.
fn render_table(frame: &mut Frame, area: Rect, app: &App) {
    let header_cells = [
        "Target", "n", "Avg", "Min", "Max", "P50", "P95", "Loss", "History",
    ]
    .iter()
    .map(|h| Cell::from(*h).style(Style::default().fg(Color::Yellow)));
    let header = Row::new(header_cells).height(1);

    // Calculate row height based on available space
    let table_inner = Block::default().borders(Borders::ALL).inner(area);
    let header_height = 1u16;
    let num_targets = app.targets.len() as u16;
    let available_height = table_inner.height.saturating_sub(header_height);
    // Each target has 2 rows, so divide by 2 * num_targets
    let row_height = if num_targets > 0 {
        (available_height / (num_targets * 2)).max(1)
    } else {
        1
    };

    let rows: Vec<Row> = app
        .targets
        .iter()
        .zip(app.stats.iter())
        .enumerate()
        .flat_map(|(idx, (target, stats))| {
            let is_selected = idx == app.selected;
            create_target_rows(
                target.name.as_str(),
                &target.addr.to_string(),
                stats,
                is_selected,
                row_height,
            )
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(26), // Target
            Constraint::Length(8),  // n
            Constraint::Length(8),  // Avg
            Constraint::Length(8),  // Min
            Constraint::Length(8),  // Max
            Constraint::Length(8),  // P50
            Constraint::Length(8),  // P95
            Constraint::Length(14), // Loss
            Constraint::Min(20),    // History sparkline
        ],
    )
    .header(header)
    .block(Block::default().borders(Borders::ALL).title("Targets"));

    frame.render_widget(table, area);

    // Render sparklines in a second pass
    render_sparklines(frame, area, app);
}

/// Creates two table rows for a target: window stats and all-time stats.
fn create_target_rows<'a>(
    name: &str,
    addr: &str,
    stats: &TargetStats,
    selected: bool,
    row_height: u16,
) -> Vec<Row<'a>> {
    let (base_style, dim_color) = if selected {
        (
            Style::default().bg(Color::Indexed(236)),
            Color::Indexed(245),
        ) // Subtle dark bg, lighter gray text
    } else {
        (Style::default(), Color::DarkGray)
    };

    let loss_color = |loss: f64| {
        if loss > 10.0 {
            Color::Red
        } else if loss > 1.0 {
            Color::Yellow
        } else {
            Color::Green
        }
    };

    // Format packet loss as "count (pct%)"
    let format_loss = |lost: u64, loss_pct: f64| -> String {
        if loss_pct == 0.0 {
            format!("{} (0%)", lost)
        } else if loss_pct < 1.0 {
            format!("{} ({:.2}%)", lost, loss_pct)
        } else {
            format!("{} ({:.1}%)", lost, loss_pct)
        }
    };

    let (window_lost, window_loss_pct) = stats.window_packet_loss();
    let (all_time_lost, all_time_loss_pct) = stats.all_time_packet_loss();

    // Row 1: Window stats (recent)
    let window_row = Row::new(vec![
        Cell::from(name.to_string()).style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from(format!(
            "last {}",
            format_count(stats.window_count() as u64)
        )),
        Cell::from(format_duration_opt(stats.average())),
        Cell::from(format_duration_opt(stats.min())),
        Cell::from(format_duration_opt(stats.max())),
        Cell::from(format_duration_opt(stats.p50())),
        Cell::from(format_duration_opt(stats.p95())),
        Cell::from(format_loss(window_lost, window_loss_pct))
            .style(Style::default().fg(loss_color(window_loss_pct))),
        Cell::from(""), // Sparkline placeholder
    ])
    .style(base_style)
    .height(row_height);

    // Row 2: All-time stats
    let all_time = &stats.all_time;
    let dim = Style::default().fg(dim_color);
    let all_time_row = Row::new(vec![
        Cell::from(format!("└ {}", addr)).style(dim),
        Cell::from(format!("all {}", format_count(stats.sent))).style(dim),
        Cell::from(format_duration_opt(all_time.average())).style(dim),
        Cell::from(format_duration_opt(all_time.min)).style(dim),
        Cell::from(format_duration_opt(all_time.max)).style(dim),
        Cell::from(format_duration_opt(all_time.p50())).style(dim),
        Cell::from(format_duration_opt(all_time.p95())).style(dim),
        Cell::from(format_loss(all_time_lost, all_time_loss_pct)).style(
            Style::default()
                .fg(loss_color(all_time_loss_pct))
                .add_modifier(Modifier::DIM),
        ),
        Cell::from(""), // Sparkline placeholder
    ])
    .style(base_style)
    .height(row_height);

    vec![window_row, all_time_row]
}

/// Renders sparklines for each target.
fn render_sparklines(frame: &mut Frame, area: Rect, app: &App) {
    let table_inner = Block::default().borders(Borders::ALL).inner(area);

    let header_height = 1u16;
    let num_targets = app.stats.len() as u16;

    // Calculate row height to match table layout (must be consistent with render_table)
    let available_height = table_inner.height.saturating_sub(header_height);
    let row_height = if num_targets > 0 {
        (available_height / (num_targets * 2)).max(1)
    } else {
        1
    };
    let rows_per_target = row_height * 2;

    for (idx, stats) in app.stats.iter().enumerate() {
        // Sparkline goes on the first row of each target pair
        let y = table_inner.y + header_height + (idx as u16 * rows_per_target);
        if y >= table_inner.y + table_inner.height {
            break;
        }

        // Sparkline column starts after the other columns
        // Width: 26 + 8 + 8 + 8 + 8 + 8 + 8 + 14 = 88
        // Add offset to avoid rendering artifacts on bottom rows
        let sparkline_offset = 8u16;
        let x = table_inner.x + 88 + sparkline_offset;
        let width = table_inner.width.saturating_sub(88 + sparkline_offset);

        if width > 0 {
            // Sparkline spans available rows for this target
            let sparkline_height = rows_per_target.min(table_inner.y + table_inner.height - y);
            let sparkline_area = Rect::new(x, y, width, sparkline_height);
            let data = stats.sparkline_data();

            // Take only the last `width` samples
            let display_data: Vec<u64> = data
                .iter()
                .rev()
                .take(width as usize)
                .rev()
                .copied()
                .collect();

            let sparkline = Sparkline::default()
                .data(&display_data)
                .style(Style::default().fg(Color::Cyan));

            frame.render_widget(sparkline, sparkline_area);
        }
    }
}

/// Renders the footer with help text.
fn render_footer(frame: &mut Frame, area: Rect, mode: ViewMode) {
    let spans = match mode {
        ViewMode::List => vec![
            Span::styled("q", Style::default().fg(Color::Yellow)),
            Span::raw(" quit  "),
            Span::styled("↑/↓", Style::default().fg(Color::Yellow)),
            Span::raw(" navigate  "),
            Span::styled("Enter", Style::default().fg(Color::Yellow)),
            Span::raw(" details  "),
            Span::styled("r", Style::default().fg(Color::Yellow)),
            Span::raw(" reset"),
        ],
        ViewMode::Detail => vec![
            Span::styled("Esc", Style::default().fg(Color::Yellow)),
            Span::raw(" back  "),
            Span::styled("↑/↓", Style::default().fg(Color::Yellow)),
            Span::raw(" prev/next target  "),
            Span::styled("q", Style::default().fg(Color::Yellow)),
            Span::raw(" quit  "),
            Span::styled("r", Style::default().fg(Color::Yellow)),
            Span::raw(" reset"),
        ],
    };

    let help = Paragraph::new(Line::from(spans)).block(Block::default().borders(Borders::TOP));

    frame.render_widget(help, area);
}

/// Renders the detail view for a single target.
fn render_detail_view(frame: &mut Frame, app: &App) {
    let (target, stats) = match app.selected_target() {
        Some(t) => t,
        None => return,
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Length(8), // Top section: Quality + Percentiles
            Constraint::Length(6), // Histogram
            Constraint::Min(6),    // Large sparkline
            Constraint::Length(5), // Packet loss details
            Constraint::Length(3), // Footer
        ])
        .split(frame.area());

    let subtitle = format!("{} ({})", target.name, target.addr);
    render_header(frame, chunks[0], Some(&subtitle), app);
    render_detail_top(frame, chunks[1], stats);
    render_histogram(frame, chunks[2], stats);
    render_large_sparkline(frame, chunks[3], stats);
    render_loss_details(frame, chunks[4], stats);
    render_footer(frame, chunks[5], ViewMode::Detail);
}

/// Renders the top section with quality score and percentiles.
fn render_detail_top(frame: &mut Frame, area: Rect, stats: &TargetStats) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);

    // Left: Quality metrics
    let (grade, grade_desc) = stats.quality_grade().unwrap_or(("-", "N/A"));
    let mos = stats
        .mos_score()
        .map(|m| format!("{:.1}", m))
        .unwrap_or("-".to_string());
    let jitter = format_duration_opt(stats.jitter());

    let grade_color = match grade {
        "A" => Color::Green,
        "B" => Color::LightGreen,
        "C" => Color::Yellow,
        "D" => Color::LightRed,
        _ => Color::Red,
    };

    let quality_text = vec![
        Line::from(vec![
            Span::raw("Quality: "),
            Span::styled(
                format!("{} ({})", grade, grade_desc),
                Style::default()
                    .fg(grade_color)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::raw("MOS Score: "),
            Span::styled(mos, Style::default().fg(Color::Cyan)),
        ]),
        Line::from(vec![
            Span::raw("Jitter: "),
            Span::styled(jitter, Style::default().fg(Color::Cyan)),
        ]),
        Line::from(vec![
            Span::raw("Samples: "),
            Span::styled(format!("{}", stats.sent), Style::default().fg(Color::Cyan)),
        ]),
        Line::from(vec![
            Span::raw("Uptime: "),
            Span::styled(
                format_elapsed(stats.elapsed()),
                Style::default().fg(Color::Cyan),
            ),
        ]),
    ];

    let quality_widget =
        Paragraph::new(quality_text).block(Block::default().borders(Borders::ALL).title("Quality"));
    frame.render_widget(quality_widget, chunks[0]);

    // Right: Percentiles
    let all_time = &stats.all_time;
    let percentile_text = vec![
        Line::from(vec![Span::styled(
            "Window (recent)",
            Style::default().add_modifier(Modifier::BOLD),
        )]),
        Line::from(format!(
            "  Min: {}  P50: {}  P95: {}  Max: {}",
            format_duration_opt(stats.min()),
            format_duration_opt(stats.p50()),
            format_duration_opt(stats.p95()),
            format_duration_opt(stats.max()),
        )),
        Line::from(""),
        Line::from(vec![Span::styled(
            "All-time",
            Style::default().add_modifier(Modifier::BOLD),
        )]),
        Line::from(format!(
            "  Min: {}  P50: {}  P95: {}  Max: {}",
            format_duration_opt(all_time.min),
            format_duration_opt(all_time.p50()),
            format_duration_opt(all_time.p95()),
            format_duration_opt(all_time.max),
        )),
    ];

    let percentile_widget = Paragraph::new(percentile_text)
        .block(Block::default().borders(Borders::ALL).title("Percentiles"));
    frame.render_widget(percentile_widget, chunks[1]);
}

/// Renders a histogram of latency distribution.
fn render_histogram(frame: &mut Frame, area: Rect, stats: &TargetStats) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Latency Distribution");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if let Some((boundaries, counts)) = stats.histogram(12) {
        let max_count = counts.iter().max().copied().unwrap_or(1);

        // Determine label precision based on bucket size
        let bucket_size = if boundaries.len() >= 2 {
            boundaries[1] - boundaries[0]
        } else {
            1.0
        };
        let precision = if bucket_size < 1.0 { 1 } else { 0 };

        // Create labels with appropriate precision
        let labels: Vec<String> = boundaries
            .iter()
            .map(|b| format!("{:.prec$}", b, prec = precision))
            .collect();

        // Build bar data with labels
        let bar_data: Vec<(String, u64)> = labels
            .into_iter()
            .zip(counts.iter())
            .map(|(l, c)| (l, *c))
            .collect();

        // Render as ASCII art since BarChart is tricky with dynamic labels
        let bar_width = inner.width as usize / bar_data.len().max(1);
        let height = inner.height.saturating_sub(1) as usize;

        let mut lines: Vec<Line> = Vec::new();

        // Build histogram rows from top to bottom
        for row in (0..height).rev() {
            let threshold = (row as f64 / height as f64) * max_count as f64;
            let mut spans: Vec<Span> = Vec::new();

            for (_label, count) in &bar_data {
                let filled = *count as f64 >= threshold;
                let bar_char = if filled { "█" } else { " " };
                spans.push(Span::styled(
                    format!("{:^width$}", bar_char, width = bar_width),
                    Style::default().fg(Color::Cyan),
                ));
            }
            lines.push(Line::from(spans));
        }

        // Add labels at bottom
        let label_spans: Vec<Span> = bar_data
            .iter()
            .map(|(label, _)| {
                Span::styled(
                    format!("{:^width$}", label, width = bar_width),
                    Style::default().fg(Color::DarkGray),
                )
            })
            .collect();
        lines.push(Line::from(label_spans));

        let para = Paragraph::new(lines);
        frame.render_widget(para, inner);
    } else {
        let no_data = Paragraph::new("No data yet...").style(Style::default().fg(Color::DarkGray));
        frame.render_widget(no_data, inner);
    }
}

/// Renders a large sparkline for the detail view.
fn render_large_sparkline(frame: &mut Frame, area: Rect, stats: &TargetStats) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Recent History");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let data = stats.sparkline_data();
    let display_data: Vec<u64> = data
        .iter()
        .rev()
        .take(inner.width as usize)
        .rev()
        .copied()
        .collect();

    let sparkline = Sparkline::default()
        .data(&display_data)
        .style(Style::default().fg(Color::Cyan));

    frame.render_widget(sparkline, inner);
}

/// Renders packet loss details.
fn render_loss_details(frame: &mut Frame, area: Rect, stats: &TargetStats) {
    let lost = stats.sent - stats.received;
    let loss_pct = stats.packet_loss();

    let time_since_loss = stats
        .time_since_last_loss()
        .map(format_elapsed)
        .unwrap_or_else(|| "never".to_string());

    let loss_color = if loss_pct > 10.0 {
        Color::Red
    } else if loss_pct > 1.0 {
        Color::Yellow
    } else {
        Color::Green
    };

    let loss_text = vec![
        Line::from(vec![
            Span::raw("Total Lost: "),
            Span::styled(
                format!("{} ({:.2}%)", lost, loss_pct),
                Style::default().fg(loss_color),
            ),
            Span::raw("  │  "),
            Span::raw("Current Streak: "),
            Span::styled(
                format!("{}", stats.current_streak),
                Style::default().fg(Color::Green),
            ),
            Span::raw("  │  "),
            Span::raw("Best Streak: "),
            Span::styled(
                format!("{}", stats.longest_streak),
                Style::default().fg(Color::Cyan),
            ),
        ]),
        Line::from(vec![
            Span::raw("Last Loss: "),
            Span::styled(
                format!("{} ago", time_since_loss),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
    ];

    let loss_widget = Paragraph::new(loss_text)
        .block(Block::default().borders(Borders::ALL).title("Packet Loss"));
    frame.render_widget(loss_widget, area);
}

/// Renders the replay view.
pub fn render_replay(
    frame: &mut Frame,
    targets: &[Target],
    stats: &[TargetStats],
    replay: &ReplayState,
    selected: usize,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Length(3), // Progress bar
            Constraint::Min(10),   // Main table
            Constraint::Length(3), // Footer/help
        ])
        .split(frame.area());

    render_replay_header(frame, chunks[0], replay);
    render_replay_progress(frame, chunks[1], replay);
    render_replay_table(frame, chunks[2], targets, stats, selected);
    render_replay_footer(frame, chunks[3], replay);
}

/// Renders the replay header.
fn render_replay_header(frame: &mut Frame, area: Rect, replay: &ReplayState) {
    let status = if replay.finished {
        Span::styled("FINISHED", Style::default().fg(Color::Green))
    } else if replay.paused {
        Span::styled("PAUSED", Style::default().fg(Color::Yellow))
    } else {
        Span::styled("▶ PLAYING", Style::default().fg(Color::Cyan))
    };

    let speed_str = format!("{}x", replay.speed());

    let log_time = replay
        .current_log_time()
        .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "-".to_string());

    let spans = vec![
        Span::styled(
            "ptop",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" │ "),
        Span::styled(
            "REPLAY",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" │ "),
        status,
        Span::raw(" │ "),
        Span::styled(speed_str, Style::default().fg(Color::Yellow)),
        Span::raw(" │ "),
        Span::styled(log_time, Style::default().fg(Color::DarkGray)),
    ];

    let header = Paragraph::new(Line::from(spans)).block(Block::default().borders(Borders::BOTTOM));

    frame.render_widget(header, area);
}

/// Renders the replay progress bar.
fn render_replay_progress(frame: &mut Frame, area: Rect, replay: &ReplayState) {
    let progress = replay.progress() / 100.0;
    let label = format!(
        "{}/{} events ({:.1}%)",
        replay.current_event(),
        replay.total_events(),
        replay.progress()
    );

    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title("Progress"))
        .gauge_style(Style::default().fg(Color::Cyan))
        .ratio(progress.min(1.0))
        .label(label);

    frame.render_widget(gauge, area);
}

/// Renders the replay table (similar to main table but without app state).
fn render_replay_table(
    frame: &mut Frame,
    area: Rect,
    targets: &[Target],
    stats: &[TargetStats],
    selected: usize,
) {
    let header_cells = [
        "Target", "n", "Avg", "Min", "Max", "P50", "P95", "Loss", "History",
    ]
    .iter()
    .map(|h| Cell::from(*h).style(Style::default().fg(Color::Yellow)));
    let header = Row::new(header_cells).height(1);

    // Calculate row height based on available space
    let table_inner = Block::default().borders(Borders::ALL).inner(area);
    let header_height = 1u16;
    let num_targets = targets.len() as u16;
    let available_height = table_inner.height.saturating_sub(header_height);
    let row_height = if num_targets > 0 {
        (available_height / (num_targets * 2)).max(1)
    } else {
        1
    };

    let rows: Vec<Row> = targets
        .iter()
        .zip(stats.iter())
        .enumerate()
        .flat_map(|(idx, (target, stats))| {
            let is_selected = idx == selected;
            create_target_rows(
                target.name.as_str(),
                &target.addr.to_string(),
                stats,
                is_selected,
                row_height,
            )
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(26),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(14),
            Constraint::Min(20),
        ],
    )
    .header(header)
    .block(Block::default().borders(Borders::ALL).title("Targets"));

    frame.render_widget(table, area);

    // Render sparklines
    render_replay_sparklines(frame, area, stats);
}

/// Renders sparklines for replay mode.
fn render_replay_sparklines(frame: &mut Frame, area: Rect, stats: &[TargetStats]) {
    let table_inner = Block::default().borders(Borders::ALL).inner(area);
    let header_height = 1u16;
    let num_targets = stats.len() as u16;

    // Calculate row height to match table layout (must be consistent with render_replay_table)
    let available_height = table_inner.height.saturating_sub(header_height);
    let row_height = if num_targets > 0 {
        (available_height / (num_targets * 2)).max(1)
    } else {
        1
    };
    let rows_per_target = row_height * 2;

    for (idx, stat) in stats.iter().enumerate() {
        let y = table_inner.y + header_height + (idx as u16 * rows_per_target);
        if y >= table_inner.y + table_inner.height {
            break;
        }

        // Add offset to avoid rendering artifacts on bottom rows
        let sparkline_offset = 8u16;
        let x = table_inner.x + 88 + sparkline_offset;
        let width = table_inner.width.saturating_sub(88 + sparkline_offset);

        if width > 0 {
            // Sparkline spans available rows for this target
            let sparkline_height = rows_per_target.min(table_inner.y + table_inner.height - y);
            let sparkline_area = Rect::new(x, y, width, sparkline_height);
            let data = stat.sparkline_data();

            let display_data: Vec<u64> = data
                .iter()
                .rev()
                .take(width as usize)
                .rev()
                .copied()
                .collect();

            let sparkline = Sparkline::default()
                .data(&display_data)
                .style(Style::default().fg(Color::Cyan));

            frame.render_widget(sparkline, sparkline_area);
        }
    }
}

/// Renders the replay footer with controls.
fn render_replay_footer(frame: &mut Frame, area: Rect, _replay: &ReplayState) {
    let spans = vec![
        Span::styled("Space", Style::default().fg(Color::Yellow)),
        Span::raw(" pause  "),
        Span::styled("←/→", Style::default().fg(Color::Yellow)),
        Span::raw(" skip  "),
        Span::styled("+/-", Style::default().fg(Color::Yellow)),
        Span::raw(" speed  "),
        Span::styled("q", Style::default().fg(Color::Yellow)),
        Span::raw(" quit"),
    ];

    let help = Paragraph::new(Line::from(spans)).block(Block::default().borders(Borders::TOP));

    frame.render_widget(help, area);
}
