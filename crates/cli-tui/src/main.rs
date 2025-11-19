use anyhow::{Context, Result};
use clap::Parser;
use daemon::{config::TranscodeConfig, job::{Job, JobStatus, load_all_jobs}};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Gauge, Paragraph, Row, Table, TableState},
    Frame, Terminal,
};
use std::io::stdout;
use std::path::PathBuf;
use std::time::Duration;
use sysinfo::System;
use humansize::{format_size, DECIMAL};

struct App {
    jobs: Vec<Job>,
    system: System,
    table_state: TableState,
    should_quit: bool,
    job_state_dir: PathBuf,
    gpu_device_path: PathBuf,
}

impl App {
    fn new(job_state_dir: PathBuf, gpu_device_path: PathBuf) -> Self {
        Self {
            jobs: Vec::new(),
            system: System::new(),
            table_state: TableState::default(),
            should_quit: false,
            job_state_dir,
            gpu_device_path,
        }
    }
    
    fn get_gpu_usage(&self) -> f64 {
        // Try to read GPU utilization from sysfs
        // For Intel GPUs, check /sys/class/drm/card0/device/gpu_busy_percent
        // Or try reading from /sys/kernel/debug/dri/0/i915_frequency_info
        let paths = vec![
            "/sys/class/drm/card0/device/gpu_busy_percent",
            "/sys/class/drm/renderD128/device/gpu_busy_percent",
            "/sys/kernel/debug/dri/0/i915_frequency_info",
        ];
        
        for path_str in paths {
            if let Ok(content) = std::fs::read_to_string(path_str) {
                // Try to parse percentage from various formats
                // Format 1: Just a number "45"
                if let Ok(val) = content.trim().parse::<f64>() {
                    return val.min(100.0).max(0.0);
                }
                
                // Format 2: "GPU freq: 800 MHz, GPU busy: 45%"
                for line in content.lines() {
                    if let Some(percent_pos) = line.find("busy:") {
                        let after_busy = &line[percent_pos + 5..];
                        if let Some(pct_pos) = after_busy.find('%') {
                            let num_str = &after_busy[..pct_pos].trim();
                            if let Ok(val) = num_str.parse::<f64>() {
                                return val.min(100.0).max(0.0);
                            }
                        }
                    }
                }
            }
        }
        
        // Fallback: return 0 if we can't read GPU usage
        0.0
    }

    fn refresh(&mut self) -> Result<()> {
        // Refresh system info
        self.system.refresh_all();

        // Reload jobs
        match load_all_jobs(&self.job_state_dir) {
            Ok(jobs) => {
                self.jobs = jobs;
                // Sort by creation time (newest first)
                self.jobs.sort_by(|a, b| b.created_at.cmp(&a.created_at));
            }
            Err(_e) => {
                // Silently fail - show empty table
                // Errors are visible in the UI (empty table, status counts)
                self.jobs = Vec::new();
            }
        }

        Ok(())
    }

    fn count_by_status(&self, status: JobStatus) -> usize {
        self.jobs.iter().filter(|j| j.status == status).count()
    }
}

fn main() -> Result<()> {
    let args = Args::parse();
    
    // Load config - if no config specified, try default location first (same as daemon)
    // Store default path in a variable that lives long enough
    let default_config_path = PathBuf::from("/etc/av1d/config.json");
    
    let config_path = if let Some(ref path) = args.config {
        Some(path.as_path())
    } else if default_config_path.exists() {
        Some(default_config_path.as_path())
    } else {
        None
    };
    
    let cfg = TranscodeConfig::load_config(config_path)
        .context("Failed to load configuration")?;

    // Setup terminal
    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = stdout();
    crossterm::execute!(stdout, crossterm::terminal::EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app
    let mut app = App::new(cfg.job_state_dir.clone(), cfg.gpu_device.clone());

    // Main event loop
    loop {
        // Refresh data
        app.refresh()?;

        // Draw UI
        terminal.draw(|f| ui(f, &mut app))?;

        // Handle input
        if crossterm::event::poll(Duration::from_millis(100))? {
            if let crossterm::event::Event::Key(key) = crossterm::event::read()? {
                match key.code {
                    crossterm::event::KeyCode::Char('q') => {
                        app.should_quit = true;
                    }
                    crossterm::event::KeyCode::Char('r') => {
                        app.refresh()?;
                    }
                    _ => {}
                }
            }
        }

        if app.should_quit {
            break;
        }
    }

    // Restore terminal
    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        crossterm::terminal::LeaveAlternateScreen
    )?;

    Ok(())
}

/// AV1 transcoding daemon TUI monitor
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to configuration file (JSON or TOML)
    #[arg(short, long)]
    config: Option<PathBuf>,
}

fn ui(f: &mut Frame, app: &mut App) {
    let size = f.size();
    
    // Check minimum terminal size
    if size.height < 10 || size.width < 80 {
        let error_msg = Paragraph::new("Terminal too small! Please resize to at least 80x10.")
            .block(Block::default().borders(Borders::ALL).title("Error"))
            .style(Style::default().fg(Color::Red));
        f.render_widget(error_msg, size);
        return;
    }
    
    // Create vertical layout with explicit constraints
    // Use exact calculations to prevent overlap
    let top_height = 3;
    let bottom_height = 3;
    let available_height = size.height.saturating_sub(top_height + bottom_height);
    
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(top_height),      // Top bar: CPU/Memory (exactly 3 lines)
            Constraint::Length(available_height), // Middle: Job table (exact remaining space)
            Constraint::Length(bottom_height),    // Bottom: Status bar (exactly 3 lines)
        ])
        .split(size);
    
    // Verify chunks don't overlap
    if main_chunks.len() >= 3 {
        // Render each section in its allocated area - ensure we don't exceed bounds
        if main_chunks[0].y + main_chunks[0].height <= size.height {
            render_top_bar(f, app, main_chunks[0]);
        }
        
        if main_chunks[1].y >= main_chunks[0].y + main_chunks[0].height && 
           main_chunks[1].y + main_chunks[1].height <= main_chunks[2].y {
            render_job_table(f, app, main_chunks[1]);
        }
        
        if main_chunks[2].y >= main_chunks[1].y + main_chunks[1].height {
            render_status_bar(f, app, main_chunks[2]);
        }
    }
}

fn render_top_bar(f: &mut Frame, app: &App, area: Rect) {
    // Split top bar into three equal parts for CPU, Memory, and GPU
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(33),
            Constraint::Percentage(34),
            Constraint::Percentage(33),
        ])
        .split(area);

    // Get CPU usage and clamp to 0-100 range
    let cpu_raw = app.system.global_cpu_usage();
    let cpu_usage = if cpu_raw.is_nan() || cpu_raw.is_infinite() {
        0.0
    } else {
        cpu_raw.min(100.0).max(0.0)
    };

    // Get memory usage and clamp to 0-100 range
    let total_memory = app.system.total_memory();
    let used_memory = app.system.used_memory();
    let memory_percent = if total_memory == 0 {
        0.0
    } else {
        let percent = (used_memory as f64 / total_memory as f64) * 100.0;
        if percent.is_nan() || percent.is_infinite() {
            0.0
        } else {
            percent.min(100.0).max(0.0)
        }
    };

    // CPU gauge
    let cpu_percent_u16 = cpu_usage.min(100.0).max(0.0) as u16;
    let cpu_gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title("CPU"))
        .gauge_style(Style::default().fg(Color::Cyan))
        .percent(cpu_percent_u16)
        .label(format!("{:.1}%", cpu_usage));
    f.render_widget(cpu_gauge, chunks[0]);

    // Memory gauge
    let memory_percent_u16 = memory_percent.min(100.0).max(0.0) as u16;
    let memory_gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title("Memory"))
        .gauge_style(Style::default().fg(Color::Green))
        .percent(memory_percent_u16)
        .label(format!("{:.1}%", memory_percent));
    f.render_widget(memory_gauge, chunks[1]);

    // GPU gauge
    let gpu_usage = app.get_gpu_usage();
    let gpu_percent_u16 = gpu_usage.min(100.0).max(0.0) as u16;
    let gpu_gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title("GPU"))
        .gauge_style(Style::default().fg(Color::Magenta))
        .percent(gpu_percent_u16)
        .label(format!("{:.1}%", gpu_usage));
    f.render_widget(gpu_gauge, chunks[2]);
}

fn render_job_table(f: &mut Frame, app: &mut App, area: Rect) {
    // Ensure we have minimum space (header + 1 row + borders = 3 lines minimum)
    if area.height < 3 {
        let error_msg = Paragraph::new("Not enough space")
            .block(Block::default().borders(Borders::ALL).title("Jobs"));
        f.render_widget(error_msg, area);
        return;
    }
    
    // Calculate available rows: area.height - 2 (for header row and borders)
    // Table block has top border (1) + header (1) + bottom border (1) = 3 lines minimum
    // So data rows = area.height - 3 (for borders and header)
    let available_height = area.height as usize;
    let max_data_rows = if available_height > 3 {
        available_height.saturating_sub(3) // Subtract top border, header, and bottom border
    } else {
        0
    };
    
    // Use shorter header names to save space
    let header = Row::new(vec![
        "ST",      // STATUS
        "FILE",    // FILE
        "ORIG",    // ORIG SIZE
        "NEW",     // NEW SIZE
        "SAVE",    // SAVINGS
        "TIME",    // DURATION
        "REASON",  // REASON
    ])
    .style(Style::default().add_modifier(Modifier::BOLD))
    .height(1);

    // Build rows - limit to what fits on screen
    let rows: Vec<Row> = if app.jobs.is_empty() {
        vec![Row::new(vec![
            "No jobs".to_string(),
            format!("Dir: {}", app.job_state_dir.display()),
            "-".to_string(),
            "-".to_string(),
            "-".to_string(),
            "-".to_string(),
            "-".to_string(),
        ]).height(1)]
    } else {
        let num_rows = max_data_rows.min(20).min(app.jobs.len());
        app.jobs
            .iter()
            .take(num_rows)
            .map(|job| {
                // Use shorter status strings to save space
                let status_str = match job.status {
                    JobStatus::Pending => "PEND",
                    JobStatus::Running => "RUN",
                    JobStatus::Success => "OK",
                    JobStatus::Failed => "FAIL",
                    JobStatus::Skipped => "SKIP",
                };

                // Truncate filename more aggressively to fit better
                let file_name = job.source_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("?")
                    .to_string();
                // Truncate based on available width - will be handled by column constraint
                let file_name = truncate_string(&file_name, 50);

                let orig_size = job.original_bytes
                    .map(|b| format_size(b, DECIMAL))
                    .unwrap_or_else(|| "-".to_string());

                let new_size = job.new_bytes
                    .map(|b| format_size(b, DECIMAL))
                    .unwrap_or_else(|| "-".to_string());

                let savings = if let (Some(orig), Some(new)) = (job.original_bytes, job.new_bytes) {
                    if orig > 0 {
                        let pct = ((orig - new) as f64 / orig as f64) * 100.0;
                        format!("{:.1}%", pct)
                    } else {
                        "-".to_string()
                    }
                } else {
                    "-".to_string()
                };

                let duration = if let (Some(started), Some(finished)) = (job.started_at, job.finished_at) {
                    let dur = finished - started;
                    format!("{}s", dur.num_seconds())
                } else {
                    "-".to_string()
                };

                // Truncate reason more aggressively
                let reason = truncate_string(job.reason.as_deref().unwrap_or("-"), 30);

                Row::new(vec![
                    status_str.to_string(),
                    file_name,
                    orig_size,
                    new_size,
                    savings,
                    duration,
                    reason,
                ])
                .height(1)
            })
            .collect()
    };

    // Column widths - optimize for space efficiency
    // Use Percentage for flexible columns to fill available width
    let widths = [
        Constraint::Length(5),        // ST (PEND/RUN/etc - shorter now)
        Constraint::Percentage(40),    // FILE (largest flexible column)
        Constraint::Length(9),        // ORIG SIZE
        Constraint::Length(9),        // NEW SIZE
        Constraint::Length(7),        // SAVE
        Constraint::Length(6),        // TIME
        Constraint::Percentage(24),   // REASON (flexible, remaining space)
    ];

    let title = if app.jobs.is_empty() {
        "Jobs (0 found)".to_string()
    } else {
        format!("Jobs ({}/{})", rows.len().saturating_sub(if app.jobs.is_empty() { 0 } else { 1 }), app.jobs.len())
    };
    
    // Use minimal spacing and compact borders
    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(title))
        .column_spacing(1);

    f.render_stateful_widget(table, area, &mut app.table_state);
}

fn render_status_bar(f: &mut Frame, app: &App, area: Rect) {
    let total = app.jobs.len();
    let running = app.count_by_status(JobStatus::Running);
    let failed = app.count_by_status(JobStatus::Failed);
    let skipped = app.count_by_status(JobStatus::Skipped);
    let pending = app.count_by_status(JobStatus::Pending);
    let success = app.count_by_status(JobStatus::Success);

    // Truncate directory path if too long
    let dir_display = app.job_state_dir.display().to_string();
    let dir_short = truncate_string(&dir_display, 35);

    let status_text = format!(
        "Total: {} | Running: {} | Pending: {} | Success: {} | Failed: {} | Skipped: {} | Dir: {} | q=quit r=refresh",
        total, running, pending, success, failed, skipped, dir_short
    );

    let paragraph = Paragraph::new(status_text)
        .block(Block::default().borders(Borders::ALL).title("Status"))
        .style(Style::default())
        .wrap(ratatui::widgets::Wrap { trim: true });
    
    f.render_widget(paragraph, area);
}

fn truncate_string(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}
