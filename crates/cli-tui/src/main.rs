use anyhow::{Context, Result};
use clap::Parser;
use chrono::Utc;
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

/// Estimate space savings in GB for AV1 transcoding based on video properties
/// Uses bitrate estimation considering resolution, frame rate, and source codec efficiency
fn estimate_space_savings_gb(job: &Job) -> Option<f64> {
    let orig_bytes = job.original_bytes?;
    if orig_bytes == 0 {
        return None;
    }
    
    // Need video metadata to do proper estimation
    let (width, height, bitrate_bps, codec, frame_rate_str) = (
        job.video_width?,
        job.video_height?,
        job.video_bitrate?,
        job.video_codec.as_deref()?,
        job.video_frame_rate.as_deref()?,
    );
    
    // Parse frame rate (format: "30/1" or "29.97")
    let fps = parse_frame_rate(frame_rate_str).unwrap_or(30.0);
    
    // Calculate pixels per frame and total pixels per second
    let pixels_per_frame = (width * height) as f64;
    let pixels_per_second = pixels_per_frame * fps;
    
    // Estimate AV1 bitrate based on resolution and frame rate
    // AV1 bitrate estimation formula based on resolution and frame rate
    // Higher resolution and frame rate require more bitrate
    // Base bitrate per megapixel-second for AV1 at reasonable quality
    let base_bitrate_per_megapixel_sec = 0.5; // Mbps per megapixel-second (conservative)
    let megapixels_per_second = pixels_per_second / 1_000_000.0;
    let estimated_av1_bitrate_mbps = megapixels_per_second * base_bitrate_per_megapixel_sec;
    
    // Adjust based on source codec efficiency
    // AV1 efficiency vs source codec (bitrate reduction factor)
    let codec_efficiency_factor = match codec.to_lowercase().as_str() {
        "hevc" | "h265" => 0.55,  // AV1 uses ~55% of HEVC bitrate (45% reduction)
        "h264" | "avc" => 0.40,   // AV1 uses ~40% of H.264 bitrate (60% reduction)
        "vp9" => 0.85,            // AV1 uses ~85% of VP9 bitrate (15% reduction)
        "av1" => 1.0,             // Already AV1, no savings
        _ => 0.50,                // Conservative estimate for unknown codecs
    };
    
    // If we have source bitrate, use it for more accurate estimation
    // Otherwise use resolution-based estimation
    let source_bitrate_mbps = bitrate_bps as f64 / 1_000_000.0;
    let estimated_av1_bitrate_mbps = if source_bitrate_mbps > 0.0 {
        // Use source bitrate adjusted by codec efficiency
        source_bitrate_mbps * codec_efficiency_factor
    } else {
        // Fallback to resolution-based estimation
        estimated_av1_bitrate_mbps
    };
    
    // Calculate duration from file size and bitrate
    // Total bitrate includes video + audio + overhead
    // Estimate audio bitrate (typically 128-256 kbps for most content)
    let estimated_audio_bitrate_mbps = 0.2; // 200 kbps average
    let total_source_bitrate_mbps = if source_bitrate_mbps > 0.0 {
        source_bitrate_mbps + estimated_audio_bitrate_mbps
    } else {
        // If no source bitrate, estimate from file size
        // duration = file_size_bytes * 8 / bitrate_bps
        // We'll use a conservative estimate
        estimated_av1_bitrate_mbps * 2.0 + estimated_audio_bitrate_mbps
    };
    
    // Calculate duration in seconds
    let duration_seconds = (orig_bytes as f64 * 8.0) / (total_source_bitrate_mbps * 1_000_000.0);
    
    // Calculate estimated AV1 file size
    // Estimated size = (video_bitrate + audio_bitrate) * duration / 8
    let total_av1_bitrate_mbps = estimated_av1_bitrate_mbps + estimated_audio_bitrate_mbps;
    let estimated_av1_bytes = (total_av1_bitrate_mbps * 1_000_000.0 * duration_seconds) / 8.0;
    
    // Ensure estimated size is reasonable (not larger than original)
    let estimated_av1_bytes = estimated_av1_bytes.min(orig_bytes as f64 * 0.95);
    
    // Calculate savings
    let estimated_savings_bytes = orig_bytes as f64 - estimated_av1_bytes;
    
    // Convert to GB (1 GB = 1,000,000,000 bytes)
    Some(estimated_savings_bytes / 1_000_000_000.0)
}

/// Parse frame rate from string format (e.g., "30/1", "29.97", "60")
fn parse_frame_rate(frame_rate_str: &str) -> Option<f64> {
    // Try parsing as fraction (e.g., "30/1")
    if let Some(slash_pos) = frame_rate_str.find('/') {
        let num_str = &frame_rate_str[..slash_pos];
        let den_str = &frame_rate_str[slash_pos + 1..];
        if let (Ok(num), Ok(den)) = (num_str.parse::<f64>(), den_str.parse::<f64>()) {
            if den != 0.0 {
                return Some(num / den);
            }
        }
    }
    
    // Try parsing as decimal (e.g., "29.97")
    frame_rate_str.parse::<f64>().ok()
}

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
        use std::process::Command;
        
        // First, find which card corresponds to Intel GPU (i915 driver)
        // card0 might be ASpeed (ast driver), Intel Arc could be card1+
        let mut intel_card_num = None;
        for card_num in 0..4 {
            let driver_link = format!("/sys/class/drm/card{}/device/driver", card_num);
            if let Ok(link) = std::fs::read_link(&driver_link) {
                // The symlink points to something like ../../../../../../bus/pci/drivers/i915
                // Extract the driver name from the path
                if let Some(driver_name) = link.iter().last().and_then(|p| p.to_str()) {
                    if driver_name == "i915" {
                        intel_card_num = Some(card_num);
                        break;
                    }
                }
            }
        }
        
        // If we found Intel GPU card, try reading frequency from it
        if let Some(card_num) = intel_card_num {
            // Method 1: Try reading frequency files directly from card directory
            // Intel Arc GPUs have these files: gt_min_freq_mhz, gt_max_freq_mhz, gt_cur_freq_mhz
            let min_path = format!("/sys/class/drm/card{}/gt_min_freq_mhz", card_num);
            let max_path = format!("/sys/class/drm/card{}/gt_max_freq_mhz", card_num);
            let curr_path = format!("/sys/class/drm/card{}/gt_cur_freq_mhz", card_num);
            
            if let (Ok(min_content), Ok(max_content), Ok(curr_content)) = (
                std::fs::read_to_string(&min_path),
                std::fs::read_to_string(&max_path),
                std::fs::read_to_string(&curr_path),
            ) {
                if let (Ok(min_freq), Ok(max_freq), Ok(curr_freq)) = (
                    min_content.trim().parse::<f64>(),
                    max_content.trim().parse::<f64>(),
                    curr_content.trim().parse::<f64>(),
                ) {
                    if max_freq > min_freq && curr_freq >= min_freq {
                        let usage = ((curr_freq - min_freq) / (max_freq - min_freq)) * 100.0;
                        return usage.min(100.0).max(0.0);
                    }
                }
            }
            
            // Method 2: Try reading from gt subdirectories (gt0, gt1, etc.)
            let mut min_freq = 0.0;
            let mut max_freq = 0.0;
            let mut curr_freq = 0.0;
            
            for gt_num in 0..4 {
                let min_path = format!("/sys/class/drm/card{}/gt{}/gt_min_freq_mhz", card_num, gt_num);
                let max_path = format!("/sys/class/drm/card{}/gt{}/gt_max_freq_mhz", card_num, gt_num);
                let curr_path = format!("/sys/class/drm/card{}/gt{}/gt_cur_freq_mhz", card_num, gt_num);
                
                if let Ok(content) = std::fs::read_to_string(&min_path) {
                    if let Ok(val) = content.trim().parse::<f64>() {
                        if min_freq == 0.0 || val < min_freq {
                            min_freq = val;
                        }
                    }
                }
                if let Ok(content) = std::fs::read_to_string(&max_path) {
                    if let Ok(val) = content.trim().parse::<f64>() {
                        if val > max_freq {
                            max_freq = val;
                        }
                    }
                }
                if let Ok(content) = std::fs::read_to_string(&curr_path) {
                    if let Ok(val) = content.trim().parse::<f64>() {
                        if val > curr_freq {
                            curr_freq = val;
                        }
                    }
                }
            }
            
            // If we have frequency info from gt subdirectories, calculate usage
            if max_freq > 0.0 && curr_freq > 0.0 && min_freq > 0.0 {
                let usage = if max_freq > min_freq {
                    ((curr_freq - min_freq) / (max_freq - min_freq)) * 100.0
                } else {
                    0.0
                };
                return usage.min(100.0).max(0.0);
            }
        }
        
        // Method 2: Try reading from /sys/kernel/debug/dri/*/i915_frequency_info
        // This might have permission issues, but try anyway
        for dri_num in 0..4 {
            let debug_path = format!("/sys/kernel/debug/dri/{}/i915_frequency_info", dri_num);
            if let Ok(content) = std::fs::read_to_string(&debug_path) {
                for line in content.lines() {
                    if let Some(busy_pos) = line.to_lowercase().find("busy") {
                        let original_line = &line[busy_pos..];
                        if let Some(pct_pos) = original_line.find('%') {
                            let before_pct = &original_line[..pct_pos];
                            let parts: Vec<&str> = before_pct.split_whitespace().collect();
                            if let Some(last_part) = parts.last() {
                                if let Ok(val) = last_part.parse::<f64>() {
                                    return val.min(100.0).max(0.0);
                                }
                            }
                        }
                    }
                }
            }
        }
        
        // Method 3: Try reading from device directory
        let device_dir = "/sys/class/drm/card0/device";
        if let Ok(entries) = std::fs::read_dir(device_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    let name_lower = name.to_lowercase();
                    if name_lower.contains("busy") || name_lower.contains("utilization") || 
                       name_lower.contains("load") || name_lower.contains("usage") {
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            let trimmed = content.trim();
                            if let Ok(val) = trimmed.parse::<f64>() {
                                let pct = if val > 1.0 { val } else { val * 100.0 };
                                return pct.min(100.0).max(0.0);
                            }
                        }
                    }
                }
            }
        }
        
        // Method 4: Try using intel_gpu_top with timeout
        if let Ok(output) = Command::new("timeout")
            .args(&["1", "intel_gpu_top", "-l", "1", "-n", "1"])
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    let line_lower = line.to_lowercase();
                    if line_lower.contains("busy") {
                        let parts: Vec<&str> = line.split_whitespace().collect();
                        for part in parts {
                            if part.ends_with('%') {
                                if let Ok(val) = part.trim_end_matches('%').parse::<f64>() {
                                    return val.min(100.0).max(0.0);
                                }
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
    if size.height < 12 || size.width < 80 {
        let error_msg = Paragraph::new("Terminal too small! Please resize to at least 80x12.")
            .block(Block::default().borders(Borders::ALL).title("Error"))
            .style(Style::default().fg(Color::Red));
        f.render_widget(error_msg, size);
        return;
    }
    
    // Check if there's a running job
    let running_job = app.jobs.iter().find(|j| j.status == JobStatus::Running);
    
    // Create vertical layout with explicit constraints
    // Use exact calculations to prevent overlap
    let top_height = 3;
    let current_job_height = if running_job.is_some() { 5 } else { 0 }; // Increased to 5 lines for 3 lines of text + borders
    let bottom_height = 3;
    let available_height = size.height.saturating_sub(top_height + current_job_height + bottom_height);
    
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(top_height),           // Top bar: CPU/Memory/GPU
            Constraint::Length(current_job_height),  // Current job info (if running)
            Constraint::Length(available_height),     // Middle: Job table
            Constraint::Length(bottom_height),        // Bottom: Status bar
        ])
        .split(size);
    
    // Verify chunks don't overlap
    if main_chunks.len() >= 4 {
        let mut chunk_idx = 0;
        
        // Render top bar
        if chunk_idx < main_chunks.len() {
            render_top_bar(f, app, main_chunks[chunk_idx]);
            chunk_idx += 1;
        }
        
        // Render current job if running
        if running_job.is_some() && chunk_idx < main_chunks.len() {
            render_current_job(f, app, main_chunks[chunk_idx]);
            chunk_idx += 1;
        }
        
        // Render job table
        if chunk_idx < main_chunks.len() {
            render_job_table(f, app, main_chunks[chunk_idx]);
            chunk_idx += 1;
        }
        
        // Render status bar
        if chunk_idx < main_chunks.len() {
            render_status_bar(f, app, main_chunks[chunk_idx]);
        }
    }
}

fn render_current_job(f: &mut Frame, app: &App, area: Rect) {
    if let Some(job) = app.jobs.iter().find(|j| j.status == JobStatus::Running) {
        // Status
        let status_str = match job.status {
            JobStatus::Pending => "PEND",
            JobStatus::Running => "RUN",
            JobStatus::Success => "OK",
            JobStatus::Failed => "FAIL",
            JobStatus::Skipped => "SKIP",
        };
        
        // File name
        let file_name = job.source_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        
        // Original size
        let orig_size = job.original_bytes
            .map(|b| format_size(b, DECIMAL))
            .unwrap_or_else(|| "-".to_string());
        
        // New size
        let new_size = job.new_bytes
            .map(|b| format_size(b, DECIMAL))
            .unwrap_or_else(|| "-".to_string());
        
        // Savings percentage (actual if completed, estimated if pending/running)
        let savings = if let (Some(orig), Some(new)) = (job.original_bytes, job.new_bytes) {
            // Actual savings if transcoding is complete
            if orig > 0 {
                let pct = ((orig - new) as f64 / orig as f64) * 100.0;
                format!("{:.1}%", pct)
            } else {
                "-".to_string()
            }
        } else {
            // Estimate savings if not yet transcoded
            if let Some(savings_gb) = estimate_space_savings_gb(job) {
                format!("~{:.1}GB", savings_gb)
            } else {
                "-".to_string()
            }
        };
        
        // Duration/Elapsed time
        let duration = if let Some(started) = job.started_at {
            if let Some(finished) = job.finished_at {
                // Job finished, show total duration
                let dur = finished - started;
                format!("{}s", dur.num_seconds())
            } else {
                // Job still running, show elapsed time
                let dur = Utc::now() - started;
                format!("{}s", dur.num_seconds())
            }
        } else {
            "-".to_string()
        };
        
        // Reason
        let reason = job.reason.as_deref().unwrap_or("-");
        
        // Format as table-like display with all columns
        let info_lines = vec![
            format!("ST: {} | FILE: {}", status_str, truncate_string(&file_name, 50)),
            format!("ORIG: {} | NEW: {} | EST SAVE: {} | TIME: {}", orig_size, new_size, savings, duration),
            format!("REASON: {}", truncate_string(reason, 70)),
        ];
        
        let paragraph = Paragraph::new(info_lines.join("\n"))
            .block(Block::default()
                .borders(Borders::ALL)
                .title("▶ CURRENT JOB")
                .style(Style::default().fg(Color::Green)))
            .style(Style::default().fg(Color::Yellow));
        
        f.render_widget(paragraph, area);
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
        "EST SAVE", // ESTIMATED SAVINGS (GB) or ACTUAL SAVINGS (%)
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
                    // Actual savings if transcoding is complete
                    if orig > 0 {
                        let pct = ((orig - new) as f64 / orig as f64) * 100.0;
                        format!("{:.1}%", pct)
                    } else {
                        "-".to_string()
                    }
                } else {
                    // Estimate savings if not yet transcoded
                    if let Some(savings_gb) = estimate_space_savings_gb(job) {
                        format!("~{:.1}GB", savings_gb)
                    } else {
                        "-".to_string()
                    }
                };

                let duration = if let Some(started) = job.started_at {
                    if let Some(finished) = job.finished_at {
                        // Job finished, show total duration
                        let dur = finished - started;
                        format!("{}s", dur.num_seconds())
                    } else if job.status == JobStatus::Running {
                        // Job still running, show elapsed time
                        let dur = Utc::now() - started;
                        format!("{}s", dur.num_seconds())
                    } else {
                        "-".to_string()
                    }
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
        Constraint::Percentage(35),   // FILE (largest flexible column)
        Constraint::Length(9),        // ORIG SIZE
        Constraint::Length(9),        // NEW SIZE
        Constraint::Length(10),       // EST SAVE (wider for "~X.XGB" format)
        Constraint::Length(6),        // TIME
        Constraint::Percentage(26),   // REASON (flexible, remaining space)
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
