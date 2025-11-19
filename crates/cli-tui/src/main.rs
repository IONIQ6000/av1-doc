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
}

impl App {
    fn new(job_state_dir: PathBuf) -> Self {
        Self {
            jobs: Vec::new(),
            system: System::new(),
            table_state: TableState::default(),
            should_quit: false,
            job_state_dir,
        }
    }

    fn refresh(&mut self) -> Result<()> {
        // Refresh system info
        self.system.refresh_all();

        // Reload jobs
        match load_all_jobs(&self.job_state_dir) {
            Ok(jobs) => {
                eprintln!("DEBUG: Loaded {} jobs from {}", jobs.len(), self.job_state_dir.display());
                self.jobs = jobs;
                // Sort by creation time (newest first)
                self.jobs.sort_by(|a, b| b.created_at.cmp(&a.created_at));
            }
            Err(e) => {
                // Log error but don't crash - show empty table
                eprintln!("ERROR: Failed to load jobs from {}: {}", self.job_state_dir.display(), e);
                eprintln!("DEBUG: Directory exists: {}", self.job_state_dir.exists());
                if self.job_state_dir.exists() {
                    if let Ok(entries) = std::fs::read_dir(&self.job_state_dir) {
                        let count: usize = entries.count();
                        eprintln!("DEBUG: Found {} files in directory", count);
                    }
                }
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
    
    eprintln!("TUI: Using job state directory: {}", cfg.job_state_dir.display());
    if !cfg.job_state_dir.exists() {
        eprintln!("Warning: Job state directory does not exist: {}", cfg.job_state_dir.display());
    }

    // Setup terminal
    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = stdout();
    crossterm::execute!(stdout, crossterm::terminal::EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app
    let mut app = App::new(cfg.job_state_dir.clone());

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
    // Split top bar into two equal halves for CPU and Memory
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
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
    
    let header = Row::new(vec![
        "STATUS",
        "FILE",
        "ORIG SIZE",
        "NEW SIZE",
        "SAVINGS",
        "DURATION",
        "REASON",
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
                let status_str = match job.status {
                    JobStatus::Pending => "PENDING",
                    JobStatus::Running => "RUNNING",
                    JobStatus::Success => "SUCCESS",
                    JobStatus::Failed => "FAILED",
                    JobStatus::Skipped => "SKIPPED",
                };

                // Truncate filename
                let file_name = job.source_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("?")
                    .to_string();
                let file_name = truncate_string(&file_name, 35);

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

                let reason = truncate_string(job.reason.as_deref().unwrap_or("-"), 25);

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

    // Column widths - use fixed sizes for most, flexible for file and reason
    let widths = [
        Constraint::Length(9),   // STATUS
        Constraint::Min(20),     // FILE (flexible)
        Constraint::Length(10),  // ORIG SIZE
        Constraint::Length(10),  // NEW SIZE
        Constraint::Length(9),   // SAVINGS
        Constraint::Length(9),   // DURATION
        Constraint::Min(15),     // REASON (flexible)
    ];

    let title = if app.jobs.is_empty() {
        "Jobs (0 found)".to_string()
    } else {
        format!("Jobs ({}/{})", rows.len().saturating_sub(1), app.jobs.len())
    };
    
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
