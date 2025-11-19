use anyhow::{Context, Result};
use clap::Parser;
use daemon::{config::TranscodeConfig, job::{Job, JobStatus, load_all_jobs}};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph, Row, Table, TableState},
    Frame, Terminal,
};
use std::io::stdout;
use std::path::PathBuf;
use std::time::Duration;
use sysinfo::System;
use humansize::{format_size, DECIMAL};

/// AV1 transcoding daemon TUI monitor
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to configuration file (JSON or TOML)
    #[arg(short, long)]
    config: Option<PathBuf>,
}

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
        self.jobs = load_all_jobs(&self.job_state_dir)
            .context("Failed to load jobs")?;

        // Sort by creation time (newest first)
        self.jobs.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        Ok(())
    }

    fn count_by_status(&self, status: JobStatus) -> usize {
        self.jobs.iter().filter(|j| j.status == status).count()
    }
}

fn main() -> Result<()> {
    let args = Args::parse();
    
    // Load config (same logic as daemon)
    let cfg = TranscodeConfig::load_config(args.config.as_deref())
        .context("Failed to load configuration")?;

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

fn ui(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // System metrics
            Constraint::Min(10),   // Job table
            Constraint::Length(3), // Status bar
        ])
        .split(f.size());

    // System metrics (CPU and memory)
    render_system_metrics(f, app, chunks[0]);

    // Job table
    render_job_table(f, &mut *app, chunks[1]);

    // Status bar
    render_status_bar(f, app, chunks[2]);
}

fn render_system_metrics(f: &mut Frame, app: &App, area: Rect) {
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

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // CPU gauge - ensure value is in valid range
    let cpu_percent_u16 = cpu_usage.min(100.0).max(0.0) as u16;
    let cpu_gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title("CPU"))
        .gauge_style(Style::default().fg(Color::Cyan))
        .percent(cpu_percent_u16)
        .label(format!("{:.1}%", cpu_usage));
    f.render_widget(cpu_gauge, chunks[0]);

    // Memory gauge - ensure value is in valid range
    let memory_percent_u16 = memory_percent.min(100.0).max(0.0) as u16;
    let memory_gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title("Memory"))
        .gauge_style(Style::default().fg(Color::Green))
        .percent(memory_percent_u16)
        .label(format!("{:.1}%", memory_percent));
    f.render_widget(memory_gauge, chunks[1]);
}

fn render_job_table(f: &mut Frame, app: &mut App, area: Rect) {
    let header = Row::new(vec![
        "STATUS",
        "FILE",
        "ORIG SIZE",
        "NEW SIZE",
        "SAVINGS %",
        "DURATION",
        "REASON",
    ])
    .style(Style::default().add_modifier(Modifier::BOLD))
    .height(1);

    let rows: Vec<Row> = app.jobs
        .iter()
        .take(20) // Show top 20 jobs
        .map(|job| {
            let status_str = match job.status {
                JobStatus::Pending => "PENDING",
                JobStatus::Running => "RUNNING",
                JobStatus::Success => "SUCCESS",
                JobStatus::Failed => "FAILED",
                JobStatus::Skipped => "SKIPPED",
            };

            let file_name = job.source_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("?")
                .to_string();

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

            let reason = job.reason.as_deref().unwrap_or("-").to_string();

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
        .collect();

    let widths = [
        Constraint::Length(10),
        Constraint::Percentage(30),
        Constraint::Length(12),
        Constraint::Length(12),
        Constraint::Length(12),
        Constraint::Length(10),
        Constraint::Percentage(20),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title("Jobs"));

    f.render_stateful_widget(table, area, &mut app.table_state);
}

fn render_status_bar(f: &mut Frame, app: &App, area: Rect) {
    let total = app.jobs.len();
    let running = app.count_by_status(JobStatus::Running);
    let failed = app.count_by_status(JobStatus::Failed);
    let skipped = app.count_by_status(JobStatus::Skipped);

    let text = Line::from(vec![
        Span::styled(
            format!("Total: {} | Running: {} | Failed: {} | Skipped: {} | ", total, running, failed, skipped),
            Style::default(),
        ),
        Span::styled(
            format!("State: {} | ", app.job_state_dir.display()),
            Style::default().fg(Color::Yellow),
        ),
        Span::styled(
            "q=quit r=refresh",
            Style::default().fg(Color::Cyan),
        ),
    ]);

    let paragraph = Paragraph::new(text)
        .block(Block::default().borders(Borders::ALL).title("Status"));
    f.render_widget(paragraph, area);
}

