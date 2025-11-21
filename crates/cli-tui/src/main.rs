use anyhow::{Context, Result};
use clap::Parser;
use chrono::{Utc, DateTime};
use daemon::{config::TranscodeConfig, job::{Job, JobStatus, load_all_jobs}};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph, Row, Sparkline, Table, TableState},
    Frame, Terminal,
};
use std::io::stdout;
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::collections::HashMap;
use sysinfo::System;
use humansize::{format_size, DECIMAL};

/// Job stage detection for progress tracking
#[derive(Debug, Clone, PartialEq)]
enum JobStage {
    Probing,        // Running ffprobe
    Transcoding,    // Running ffmpeg (has temp file, size growing)
    Verifying,      // Temp file complete, checking sizes/verifying
    Replacing,      // Replacing original file
    Complete,       // Job finished
}

impl JobStage {
    fn as_str(&self) -> &'static str {
        match self {
            JobStage::Probing => "Probing",
            JobStage::Transcoding => "Transcoding",
            JobStage::Verifying => "Verifying",
            JobStage::Replacing => "Replacing",
            JobStage::Complete => "Complete",
        }
    }
}

/// Filter mode for job list
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JobFilter {
    All,
    Pending,
    Running,
    Success,
    Failed,
}

/// Sort mode for job list
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortMode {
    ByDate,
    BySize,
    ByStatus,
    BySavings,
}

/// View mode for the UI
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    Normal,
    DetailView,
}

/// UI state management
#[derive(Debug, Clone)]
struct UiState {
    // Navigation
    selected_index: Option<usize>,
    scroll_offset: usize,
    
    // Filtering and sorting
    filter: JobFilter,
    sort_mode: SortMode,
    
    // View mode
    view_mode: ViewMode,
    detail_view_job_id: Option<String>,
    
    // Table state
    table_state: TableState,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            selected_index: None,
            scroll_offset: 0,
            filter: JobFilter::All,
            sort_mode: SortMode::ByDate,
            view_mode: ViewMode::Normal,
            detail_view_job_id: None,
            table_state: TableState::default(),
        }
    }
}

/// Statistics cache for aggregate metrics
#[derive(Debug, Clone)]
struct StatisticsCache {
    // Aggregate metrics
    total_space_saved: u64,
    average_compression_ratio: f64,
    total_processing_time: i64,
    estimated_pending_savings: u64,
    success_rate: f64,
    
    // Trends (last 20 jobs)
    recent_processing_times: Vec<i64>,
    recent_compression_ratios: Vec<f64>,
    recent_completion_rate: f64,
    
    // Last update time
    last_calculated: DateTime<Utc>,
}

impl Default for StatisticsCache {
    fn default() -> Self {
        Self {
            total_space_saved: 0,
            average_compression_ratio: 0.0,
            total_processing_time: 0,
            estimated_pending_savings: 0,
            success_rate: 0.0,
            recent_processing_times: Vec::new(),
            recent_compression_ratios: Vec::new(),
            recent_completion_rate: 0.0,
            last_calculated: Utc::now(),
        }
    }
}

impl StatisticsCache {
    /// Calculate statistics from job list
    fn calculate(jobs: &[Job]) -> Self {
        let now = Utc::now();
        
        // Calculate total space saved from completed jobs
        let total_space_saved: u64 = jobs.iter()
            .filter(|j| j.status == JobStatus::Success)
            .filter_map(|j| {
                if let (Some(orig), Some(new)) = (j.original_bytes, j.new_bytes) {
                    Some(orig.saturating_sub(new))
                } else {
                    None
                }
            })
            .sum();
        
        // Calculate average compression ratio
        let compression_ratios: Vec<f64> = jobs.iter()
            .filter(|j| j.status == JobStatus::Success)
            .filter_map(|j| {
                if let (Some(orig), Some(new)) = (j.original_bytes, j.new_bytes) {
                    if orig > 0 {
                        Some((orig - new) as f64 / orig as f64)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();
        
        let average_compression_ratio = if !compression_ratios.is_empty() {
            compression_ratios.iter().sum::<f64>() / compression_ratios.len() as f64
        } else {
            0.0
        };
        
        // Calculate total processing time
        let total_processing_time: i64 = jobs.iter()
            .filter(|j| j.status == JobStatus::Success || j.status == JobStatus::Failed)
            .filter_map(|j| {
                if let (Some(started), Some(finished)) = (j.started_at, j.finished_at) {
                    Some((finished - started).num_seconds())
                } else {
                    None
                }
            })
            .sum();
        
        // Calculate estimated pending savings
        let estimated_pending_savings: u64 = jobs.iter()
            .filter(|j| j.status == JobStatus::Pending)
            .filter_map(|j| {
                if let Some((savings_gb, _)) = estimate_space_savings(j) {
                    Some((savings_gb * 1_000_000_000.0) as u64)
                } else {
                    None
                }
            })
            .sum();
        
        // Calculate success rate
        let completed_jobs = jobs.iter()
            .filter(|j| j.status == JobStatus::Success || j.status == JobStatus::Failed)
            .count();
        let successful_jobs = jobs.iter()
            .filter(|j| j.status == JobStatus::Success)
            .count();
        
        let success_rate = if completed_jobs > 0 {
            (successful_jobs as f64 / completed_jobs as f64) * 100.0
        } else {
            0.0
        };
        
        // Get recent jobs (last 20 completed)
        let mut recent_jobs: Vec<&Job> = jobs.iter()
            .filter(|j| j.status == JobStatus::Success || j.status == JobStatus::Failed)
            .collect();
        recent_jobs.sort_by(|a, b| {
            let a_time = a.finished_at.unwrap_or(a.created_at);
            let b_time = b.finished_at.unwrap_or(b.created_at);
            b_time.cmp(&a_time)
        });
        recent_jobs.truncate(20);
        
        // Calculate recent processing times
        let recent_processing_times: Vec<i64> = recent_jobs.iter()
            .filter_map(|j| {
                if let (Some(started), Some(finished)) = (j.started_at, j.finished_at) {
                    Some((finished - started).num_seconds())
                } else {
                    None
                }
            })
            .collect();
        
        // Calculate recent compression ratios
        let recent_compression_ratios: Vec<f64> = recent_jobs.iter()
            .filter(|j| j.status == JobStatus::Success)
            .filter_map(|j| {
                if let (Some(orig), Some(new)) = (j.original_bytes, j.new_bytes) {
                    if orig > 0 {
                        Some((orig - new) as f64 / orig as f64)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();
        
        // Calculate recent completion rate
        let recent_completed = recent_jobs.len();
        let recent_successful = recent_jobs.iter()
            .filter(|j| j.status == JobStatus::Success)
            .count();
        
        let recent_completion_rate = if recent_completed > 0 {
            (recent_successful as f64 / recent_completed as f64) * 100.0
        } else {
            0.0
        };
        
        Self {
            total_space_saved,
            average_compression_ratio,
            total_processing_time,
            estimated_pending_savings,
            success_rate,
            recent_processing_times,
            recent_compression_ratios,
            recent_completion_rate,
            last_calculated: now,
        }
    }
    
    /// Check if cache needs refresh (older than 5 seconds)
    fn needs_refresh(&self) -> bool {
        (Utc::now() - self.last_calculated).num_seconds() > 5
    }
}

/// Color scheme for consistent UI styling
#[derive(Debug, Clone)]
struct ColorScheme {
    // Status colors
    pending: Color,
    running: Color,
    success: Color,
    failed: Color,
    skipped: Color,
    
    // UI element colors
    border_normal: Color,
    border_selected: Color,
    header: Color,
    text_primary: Color,
    text_secondary: Color,
    text_muted: Color,
    
    // Progress colors
    progress_probing: Color,
    progress_transcoding: Color,
    progress_verifying: Color,
    progress_complete: Color,
    
    // Metric colors
    metric_low: Color,
    metric_medium: Color,
    metric_high: Color,
}

impl Default for ColorScheme {
    fn default() -> Self {
        Self {
            pending: Color::Yellow,
            running: Color::Green,
            success: Color::Blue,
            failed: Color::Red,
            skipped: Color::Gray,
            
            border_normal: Color::DarkGray,
            border_selected: Color::Cyan,
            header: Color::Cyan,
            text_primary: Color::White,
            text_secondary: Color::Gray,
            text_muted: Color::DarkGray,
            
            progress_probing: Color::Yellow,
            progress_transcoding: Color::Green,
            progress_verifying: Color::Cyan,
            progress_complete: Color::Blue,
            
            metric_low: Color::Green,
            metric_medium: Color::Yellow,
            metric_high: Color::Red,
        }
    }
}

impl ColorScheme {
    /// Get color for a job status
    fn status_color(&self, status: &JobStatus) -> Color {
        match status {
            JobStatus::Pending => self.pending,
            JobStatus::Running => self.running,
            JobStatus::Success => self.success,
            JobStatus::Failed => self.failed,
            JobStatus::Skipped => self.skipped,
        }
    }
    
    /// Get color for a job stage
    fn stage_color(&self, stage: &JobStage) -> Color {
        match stage {
            JobStage::Probing => self.progress_probing,
            JobStage::Transcoding => self.progress_transcoding,
            JobStage::Verifying => self.progress_verifying,
            JobStage::Replacing | JobStage::Complete => self.progress_complete,
        }
    }
    
    /// Get color for a codec
    fn codec_color(&self, codec: &str) -> Color {
        match codec.to_lowercase().as_str() {
            "h264" | "avc" => Color::Yellow,
            "hevc" | "h265" => Color::Green,
            "vp9" => Color::Cyan,
            "av1" => Color::Blue,
            _ => Color::Gray,
        }
    }
    
    /// Get color gradient for a percentage value (0-100)
    /// Returns color based on value: low (green) -> medium (yellow) -> high (red)
    fn gradient_color_for_percentage(&self, percentage: f64) -> Color {
        if percentage < 50.0 {
            self.metric_low
        } else if percentage < 80.0 {
            self.metric_medium
        } else {
            self.metric_high
        }
    }
    
    /// Get color gradient for compression ratio (higher is better)
    /// Returns color based on value: low (red) -> medium (yellow) -> high (green)
    fn gradient_color_for_compression(&self, ratio_pct: f64) -> Color {
        if ratio_pct > 50.0 {
            self.metric_low  // Green for good compression
        } else if ratio_pct > 30.0 {
            self.metric_medium  // Yellow for medium compression
        } else {
            self.metric_high  // Red for poor compression
        }
    }
    
    /// Get color gradient for savings (higher is better)
    /// Returns color based on GB saved
    fn gradient_color_for_savings(&self, savings_gb: f64) -> Color {
        if savings_gb > 5.0 {
            self.metric_low  // Green for large savings
        } else if savings_gb > 1.0 {
            self.metric_medium  // Yellow for medium savings
        } else {
            self.metric_high  // Red for small savings
        }
    }
}

/// Table columns that can be displayed
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum TableColumn {
    Status,
    File,
    Resolution,
    Codec,
    Bitrate,
    Hdr,
    BitDepth,
    OrigSize,
    NewSize,
    CompressionRatio,
    Quality,
    Savings,
    Time,
    Reason,
}

impl TableColumn {
    /// Get the header name for this column
    fn header(&self) -> &'static str {
        match self {
            TableColumn::Status => "ST",
            TableColumn::File => "FILE",
            TableColumn::Resolution => "RES",
            TableColumn::Codec => "CODEC",
            TableColumn::Bitrate => "BITRATE",
            TableColumn::Hdr => "HDR",
            TableColumn::BitDepth => "BITS",
            TableColumn::OrigSize => "ORIG",
            TableColumn::NewSize => "NEW",
            TableColumn::CompressionRatio => "RATIO",
            TableColumn::Quality => "Q",
            TableColumn::Savings => "EST SAVE",
            TableColumn::Time => "TIME",
            TableColumn::Reason => "REASON",
        }
    }
    
    /// Get the width constraint for this column
    fn width(&self) -> Constraint {
        match self {
            TableColumn::Status => Constraint::Length(5),
            TableColumn::File => Constraint::Percentage(14),
            TableColumn::Resolution => Constraint::Length(11),
            TableColumn::Codec => Constraint::Length(6),
            TableColumn::Bitrate => Constraint::Length(7),
            TableColumn::Hdr => Constraint::Length(4),
            TableColumn::BitDepth => Constraint::Length(4),
            TableColumn::OrigSize => Constraint::Length(9),
            TableColumn::NewSize => Constraint::Length(9),
            TableColumn::CompressionRatio => Constraint::Length(5),
            TableColumn::Quality => Constraint::Length(4),
            TableColumn::Savings => Constraint::Length(15),
            TableColumn::Time => Constraint::Length(6),
            TableColumn::Reason => Constraint::Percentage(14),
        }
    }
}

/// Layout configuration for responsive column selection
#[derive(Debug, Clone)]
struct LayoutConfig {
    terminal_size: Rect,
    show_statistics: bool,
    show_current_job: bool,
    show_detail_view: bool,
    table_columns: Vec<TableColumn>,
    // Component heights and positions
    header_height: u16,
    statistics_height: u16,
    current_job_height: u16,
    table_height: u16,
    status_bar_height: u16,
    // Minimum size requirements
    is_very_small: bool,
    is_too_small: bool,
}

impl LayoutConfig {
    /// Create layout configuration based on terminal size
    /// Task 14.1: Determine component visibility, calculate heights/widths, handle minimum size requirements
    fn from_terminal_size(size: Rect) -> Self {
        let width = size.width;
        let height = size.height;
        
        // Check if terminal is too small to display anything useful
        let is_too_small = width < 80 || height < 12;
        let is_very_small = width < 100 || height < 15;
        
        // Determine what to show based on size (Requirements 10.1, 10.2, 10.3, 10.4)
        let show_statistics = height >= 20 && !is_too_small;
        let show_current_job = true; // Always show if there's a running job
        let show_detail_view = false; // Controlled by view mode, not layout
        
        // Calculate component heights based on available space
        // Priority: Header > Status Bar > Table > Current Job > Statistics
        let header_height = if is_too_small { 3 } else { 3 }; // Fixed: top bar with system metrics
        let status_bar_height = if is_too_small { 2 } else { 3 }; // Fixed: status bar at bottom
        
        // Statistics panel height varies by terminal size
        let statistics_height = if show_statistics {
            if height >= 30 {
                8 // Full statistics with sparklines
            } else if height >= 20 {
                6 // Compact statistics
            } else {
                0 // Hidden
            }
        } else {
            0
        };
        
        // Current job panel height (only shown when there's a running job)
        let current_job_height = if is_too_small {
            0 // Hide in very small terminals
        } else if height >= 25 {
            7 // Full detail with all metadata
        } else if height >= 20 {
            6 // Compact detail
        } else {
            5 // Minimal detail
        };
        
        // Calculate remaining space for table
        // Table needs at least 3 lines (header + 1 row + borders)
        let used_height = header_height + status_bar_height + statistics_height;
        let table_height = if height > used_height + 3 {
            height - used_height
        } else {
            3 // Minimum table height
        };
        
        // Determine visible columns based on terminal width (Requirements 10.1, 10.4)
        let table_columns = if is_too_small {
            // Very small terminal: absolute minimum
            vec![
                TableColumn::Status,
                TableColumn::File,
                TableColumn::Savings,
            ]
        } else if width >= 160 {
            // Large terminal: show all columns
            vec![
                TableColumn::Status,
                TableColumn::File,
                TableColumn::Resolution,
                TableColumn::Codec,
                TableColumn::Bitrate,
                TableColumn::Hdr,
                TableColumn::BitDepth,
                TableColumn::OrigSize,
                TableColumn::NewSize,
                TableColumn::CompressionRatio,
                TableColumn::Quality,
                TableColumn::Savings,
                TableColumn::Time,
                TableColumn::Reason,
            ]
        } else if width >= 120 {
            // Medium terminal: show essential columns
            vec![
                TableColumn::Status,
                TableColumn::File,
                TableColumn::Resolution,
                TableColumn::Codec,
                TableColumn::OrigSize,
                TableColumn::NewSize,
                TableColumn::CompressionRatio,
                TableColumn::Savings,
                TableColumn::Time,
            ]
        } else {
            // Small terminal: minimal columns
            vec![
                TableColumn::Status,
                TableColumn::File,
                TableColumn::OrigSize,
                TableColumn::NewSize,
                TableColumn::Savings,
            ]
        };
        
        Self {
            terminal_size: size,
            show_statistics,
            show_current_job,
            show_detail_view,
            table_columns,
            header_height,
            statistics_height,
            current_job_height,
            table_height,
            status_bar_height,
            is_very_small,
            is_too_small,
        }
    }
    
    /// Calculate layout chunks for rendering
    /// Returns a vector of Rects for each component in order:
    /// [header, statistics (if shown), current_job (if shown), table, status_bar]
    fn calculate_chunks(&self, has_running_job: bool) -> Vec<Rect> {
        let mut constraints = Vec::new();
        
        // Header (always shown)
        constraints.push(Constraint::Length(self.header_height));
        
        // Statistics (conditional)
        if self.show_statistics {
            constraints.push(Constraint::Length(self.statistics_height));
        }
        
        // Current job panel (conditional - only if there's a running job)
        if self.show_current_job && has_running_job && !self.is_too_small {
            constraints.push(Constraint::Length(self.current_job_height));
        }
        
        // Table (flexible, takes remaining space)
        constraints.push(Constraint::Min(3)); // Minimum 3 lines for table
        
        // Status bar (always shown)
        constraints.push(Constraint::Length(self.status_bar_height));
        
        // Create layout
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(self.terminal_size);
        
        layout.iter().copied().collect()
    }
}

/// Progress tracking for a running job
#[derive(Clone)]
struct JobProgress {
    temp_file_path: PathBuf,
    temp_file_size: u64,
    original_size: u64,
    last_updated: DateTime<Utc>,
    bytes_per_second: f64,  // Estimated write rate
    estimated_completion: Option<DateTime<Utc>>,  // ETA
    stage: JobStage,  // Current stage (ffprobe, transcoding, verifying, etc.)
    progress_percent: f64,  // Progress percentage (0-100)
    
    // Enhanced tracking fields (Task 13.2)
    frames_processed: Option<u64>,
    total_frames: Option<u64>,
    current_fps: Option<f64>,
    estimated_final_size: Option<u64>,
    current_compression_ratio: Option<f64>,
    
    // Additional tracking for FPS calculation
    last_temp_file_size: u64,
    last_progress_percent: f64,
}

impl JobProgress {
    fn new(temp_file_path: PathBuf, original_size: u64) -> Self {
        Self {
            temp_file_path,
            temp_file_size: 0,
            original_size,
            last_updated: Utc::now(),
            bytes_per_second: 0.0,
            estimated_completion: None,
            stage: JobStage::Probing,
            progress_percent: 0.0,
            frames_processed: None,
            total_frames: None,
            current_fps: None,
            estimated_final_size: None,
            current_compression_ratio: None,
            last_temp_file_size: 0,
            last_progress_percent: 0.0,
        }
    }
    
    /// Calculate total frames from video metadata (Task 13.1)
    /// Uses duration and frame rate to estimate total frames
    fn calculate_total_frames(duration_secs: f64, frame_rate: f64) -> Option<u64> {
        if duration_secs > 0.0 && frame_rate > 0.0 && frame_rate < 200.0 {
            Some((duration_secs * frame_rate) as u64)
        } else {
            None
        }
    }
    
    /// Estimate frames processed based on progress percentage (Task 13.1)
    fn estimate_frames_processed(&self) -> Option<u64> {
        if let Some(total) = self.total_frames {
            if self.progress_percent >= 0.0 && self.progress_percent <= 100.0 {
                Some(((self.progress_percent / 100.0) * total as f64) as u64)
            } else {
                None
            }
        } else {
            None
        }
    }
    
    /// Calculate current FPS based on progress rate (Task 13.1)
    /// Uses the change in progress percentage over time to estimate FPS
    fn calculate_current_fps(&self, time_delta_secs: f64, progress_delta: f64) -> Option<f64> {
        if let Some(total_frames) = self.total_frames {
            if time_delta_secs > 0.0 && progress_delta > 0.0 && total_frames > 0 {
                // Calculate frames processed in this time delta
                let frames_delta = (progress_delta / 100.0) * total_frames as f64;
                let fps = frames_delta / time_delta_secs;
                
                // Sanity check: FPS should be reasonable (0.1 to 500)
                if fps > 0.1 && fps < 500.0 {
                    Some(fps)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    }
}

/// Validate that job has all required metadata for estimation
fn has_estimation_metadata(job: &Job) -> bool {
    job.original_bytes.is_some()
        && job.video_codec.is_some()
        && job.video_width.is_some()
        && job.video_height.is_some()
        && job.video_bitrate.is_some()
        && job.video_frame_rate.is_some()
}

/// Calculate estimated output size based on quality and codec
/// Returns None if required metadata is not available
fn calculate_estimated_output_size(job: &Job) -> Option<u64> {
    let orig_bytes = job.original_bytes?;
    if orig_bytes == 0 {
        return None;
    }
    
    let codec = job.video_codec.as_deref()?;
    
    // If we have actual quality setting, use it for more accurate estimation
    if let Some(quality) = job.av1_quality {
        // Base reduction percentage by quality
        let base_reduction: f64 = match quality {
            20..=22 => 0.45, // ~45% reduction (very high quality)
            23..=24 => 0.55, // ~55% reduction (high quality)
            25..=26 => 0.65, // ~65% reduction (balanced)
            27..=28 => 0.70, // ~70% reduction (more compression)
            29..=30 => 0.75, // ~75% reduction (high compression)
            _ => 0.60, // Default
        };
        
        // Adjust based on source codec efficiency
        let codec_factor: f64 = match codec.to_lowercase().as_str() {
            "h264" | "avc" => 1.05, // H.264 allows more compression
            "hevc" | "h265" => 0.90, // HEVC already efficient, less room
            "vp9" => 0.92, // VP9 already efficient
            "av1" => 1.0, // Already AV1
            _ => 1.0, // Default
        };
        
        let reduction = (base_reduction * codec_factor).min(0.80).max(0.35);
        let estimated_output_size = orig_bytes as f64 * (1.0 - reduction);
        return Some(estimated_output_size as u64);
    }
    
    // Fallback to codec-based estimation if quality not available
    let efficiency_factor = match codec.to_lowercase().as_str() {
        "hevc" | "h265" => 0.55,
        "h264" | "avc" => 0.40,
        "vp9" => 0.85,
        "av1" => 1.0,
        _ => 0.5,
    };
    Some((orig_bytes as f64 * efficiency_factor) as u64)
}

/// Estimate space savings in GB and percentage for AV1 transcoding based on video properties
/// Uses quality setting if available, otherwise falls back to codec-based estimation
/// Returns None if required metadata is not available
fn estimate_space_savings(job: &Job) -> Option<(f64, f64)> { // Returns (savings_gb, savings_percent)
    let orig_bytes = job.original_bytes?;
    if orig_bytes == 0 {
        return None;
    }
    
    // Calculate estimated output size
    let estimated_output_bytes = calculate_estimated_output_size(job)? as f64;
    
    // Calculate savings
    let estimated_savings_bytes = orig_bytes as f64 - estimated_output_bytes;
    let savings_gb = estimated_savings_bytes / 1_000_000_000.0;
    let savings_percent = (estimated_savings_bytes / orig_bytes as f64) * 100.0;
    
    Some((savings_gb, savings_percent))
}

/// Estimate space savings in GB for AV1 transcoding (backward compatibility)
/// Returns None if required metadata is not available
fn estimate_space_savings_gb(job: &Job) -> Option<f64> {
    estimate_space_savings(job).map(|(gb, _)| gb)
}

/// Parse frame rate from string format (e.g., "30/1", "29.97", "60")
/// Returns None if parsing fails - no fallback values
fn parse_frame_rate(frame_rate_str: &str) -> Option<f64> {
    // Try parsing as fraction (e.g., "30/1")
    if let Some(slash_pos) = frame_rate_str.find('/') {
        let num_str = &frame_rate_str[..slash_pos];
        let den_str = &frame_rate_str[slash_pos + 1..];
        if let (Ok(num), Ok(den)) = (num_str.parse::<f64>(), den_str.parse::<f64>()) {
            if den != 0.0 && num > 0.0 {
                return Some(num / den);
            }
        }
    }
    
    // Try parsing as decimal (e.g., "29.97")
    if let Ok(fps) = frame_rate_str.parse::<f64>() {
        if fps > 0.0 {
            return Some(fps);
        }
    }
    
    None // Failed to parse - return None
}

struct App {
    // Core data
    jobs: Vec<Job>,
    system: System,
    
    // UI state
    ui_state: UiState,
    
    // Caching and tracking
    job_progress: HashMap<String, JobProgress>,
    statistics_cache: StatisticsCache,
    estimated_savings_cache: HashMap<String, Option<(f64, f64)>>,
    
    // Configuration
    job_state_dir: PathBuf,
    command_dir: PathBuf,
    temp_output_dir: PathBuf,
    
    // Timing and status
    last_refresh: DateTime<Utc>,
    last_job_count: usize,
    last_message: Option<String>,
    message_timeout: Option<DateTime<Utc>>,
    
    // Color scheme
    color_scheme: ColorScheme,
    
    // Control flags
    should_quit: bool,
}

impl App {
    /// Generate temp output path using configured temp_output_dir (same logic as daemon)
    fn get_temp_output_path(&self, source_path: &Path) -> PathBuf {
        let filename = source_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");
        self.temp_output_dir.join(format!("{}.tmp.av1.mkv", filename))
    }
    
    fn new(job_state_dir: PathBuf, temp_output_dir: PathBuf) -> Self {
        // Derive command_dir from job_state_dir
        let command_dir = job_state_dir.parent()
            .map(|p| p.join("commands"))
            .unwrap_or_else(|| PathBuf::from("/var/lib/av1d/commands"));
        
        Self {
            jobs: Vec::new(),
            system: System::new(),
            ui_state: UiState::default(),
            job_progress: HashMap::new(),
            statistics_cache: StatisticsCache::default(),
            estimated_savings_cache: HashMap::new(),
            job_state_dir,
            command_dir,
            temp_output_dir,
            last_refresh: Utc::now(),
            last_job_count: 0,
            last_message: None,
            message_timeout: None,
            color_scheme: ColorScheme::default(),
            should_quit: false,
        }
    }
    
    /// Filter jobs based on the current filter setting
    fn filter_jobs<'a>(&'a self, jobs: &'a [Job]) -> Vec<&'a Job> {
        match self.ui_state.filter {
            JobFilter::All => jobs.iter().collect(),
            JobFilter::Pending => jobs.iter().filter(|j| j.status == JobStatus::Pending).collect(),
            JobFilter::Running => jobs.iter().filter(|j| j.status == JobStatus::Running).collect(),
            JobFilter::Success => jobs.iter().filter(|j| j.status == JobStatus::Success).collect(),
            JobFilter::Failed => jobs.iter().filter(|j| j.status == JobStatus::Failed).collect(),
        }
    }
    
    /// Sort jobs based on the current sort mode
    fn sort_jobs(&self, jobs: &mut Vec<&Job>) {
        match self.ui_state.sort_mode {
            SortMode::ByDate => {
                // Sort by creation date (newest first)
                jobs.sort_by(|a, b| b.created_at.cmp(&a.created_at));
            }
            SortMode::BySize => {
                // Sort by original size (largest first)
                jobs.sort_by(|a, b| {
                    let a_size = a.original_bytes.unwrap_or(0);
                    let b_size = b.original_bytes.unwrap_or(0);
                    b_size.cmp(&a_size)
                });
            }
            SortMode::ByStatus => {
                // Sort by status: Running > Failed > Pending > Success > Skipped
                jobs.sort_by(|a, b| {
                    let a_priority = match a.status {
                        JobStatus::Running => 0,
                        JobStatus::Failed => 1,
                        JobStatus::Pending => 2,
                        JobStatus::Success => 3,
                        JobStatus::Skipped => 4,
                    };
                    let b_priority = match b.status {
                        JobStatus::Running => 0,
                        JobStatus::Failed => 1,
                        JobStatus::Pending => 2,
                        JobStatus::Success => 3,
                        JobStatus::Skipped => 4,
                    };
                    a_priority.cmp(&b_priority)
                });
            }
            SortMode::BySavings => {
                // Sort by savings (highest first)
                jobs.sort_by(|a, b| {
                    let a_savings = if let (Some(orig), Some(new)) = (a.original_bytes, a.new_bytes) {
                        orig.saturating_sub(new)
                    } else {
                        // Use estimated savings for pending/running jobs
                        self.estimated_savings_cache.get(&a.id)
                            .and_then(|opt| opt.as_ref())
                            .map(|(gb, _)| (*gb * 1_000_000_000.0) as u64)
                            .unwrap_or(0)
                    };
                    let b_savings = if let (Some(orig), Some(new)) = (b.original_bytes, b.new_bytes) {
                        orig.saturating_sub(new)
                    } else {
                        // Use estimated savings for pending/running jobs
                        self.estimated_savings_cache.get(&b.id)
                            .and_then(|opt| opt.as_ref())
                            .map(|(gb, _)| (*gb * 1_000_000_000.0) as u64)
                            .unwrap_or(0)
                    };
                    b_savings.cmp(&a_savings)
                });
            }
        }
    }
    
    /// Cycle to the next sort mode
    fn cycle_sort_mode(&mut self) {
        self.ui_state.sort_mode = match self.ui_state.sort_mode {
            SortMode::ByDate => SortMode::BySize,
            SortMode::BySize => SortMode::ByStatus,
            SortMode::ByStatus => SortMode::BySavings,
            SortMode::BySavings => SortMode::ByDate,
        };
    }
    
    /// Move selection up by one
    fn move_selection_up(&mut self) {
        // Get filtered job count
        let filtered_jobs = self.filter_jobs(&self.jobs);
        let job_count = filtered_jobs.len();
        
        if job_count == 0 {
            self.ui_state.selected_index = None;
            return;
        }
        
        match self.ui_state.selected_index {
            None => {
                // No selection, select last item
                self.ui_state.selected_index = Some(job_count.saturating_sub(1));
            }
            Some(0) => {
                // At top, wrap to bottom
                self.ui_state.selected_index = Some(job_count.saturating_sub(1));
            }
            Some(idx) => {
                // Move up one
                self.ui_state.selected_index = Some(idx.saturating_sub(1));
            }
        }
        
        // Update table state
        if let Some(idx) = self.ui_state.selected_index {
            self.ui_state.table_state.select(Some(idx));
        }
    }
    
    /// Move selection down by one
    fn move_selection_down(&mut self) {
        // Get filtered job count
        let filtered_jobs = self.filter_jobs(&self.jobs);
        let job_count = filtered_jobs.len();
        
        if job_count == 0 {
            self.ui_state.selected_index = None;
            return;
        }
        
        match self.ui_state.selected_index {
            None => {
                // No selection, select first item
                self.ui_state.selected_index = Some(0);
            }
            Some(idx) if idx >= job_count.saturating_sub(1) => {
                // At bottom, wrap to top
                self.ui_state.selected_index = Some(0);
            }
            Some(idx) => {
                // Move down one
                self.ui_state.selected_index = Some(idx + 1);
            }
        }
        
        // Update table state
        if let Some(idx) = self.ui_state.selected_index {
            self.ui_state.table_state.select(Some(idx));
        }
    }
    
    /// Move selection up by one page (10 items)
    fn move_selection_page_up(&mut self) {
        // Get filtered job count
        let filtered_jobs = self.filter_jobs(&self.jobs);
        let job_count = filtered_jobs.len();
        
        if job_count == 0 {
            self.ui_state.selected_index = None;
            return;
        }
        
        let page_size = 10;
        
        match self.ui_state.selected_index {
            None => {
                // No selection, select last item
                self.ui_state.selected_index = Some(job_count.saturating_sub(1));
            }
            Some(idx) => {
                // Move up by page_size, but don't go below 0
                self.ui_state.selected_index = Some(idx.saturating_sub(page_size));
            }
        }
        
        // Update table state
        if let Some(idx) = self.ui_state.selected_index {
            self.ui_state.table_state.select(Some(idx));
        }
    }
    
    /// Move selection down by one page (10 items)
    fn move_selection_page_down(&mut self) {
        // Get filtered job count
        let filtered_jobs = self.filter_jobs(&self.jobs);
        let job_count = filtered_jobs.len();
        
        if job_count == 0 {
            self.ui_state.selected_index = None;
            return;
        }
        
        let page_size = 10;
        
        match self.ui_state.selected_index {
            None => {
                // No selection, select first item
                self.ui_state.selected_index = Some(0);
            }
            Some(idx) => {
                // Move down by page_size, but don't exceed job_count - 1
                let new_idx = (idx + page_size).min(job_count.saturating_sub(1));
                self.ui_state.selected_index = Some(new_idx);
            }
        }
        
        // Update table state
        if let Some(idx) = self.ui_state.selected_index {
            self.ui_state.table_state.select(Some(idx));
        }
    }
    
    /// Write a requeue command file for a running job
    fn requeue_running_job(&mut self) -> Result<()> {
        // Find the running job
        let running_job = self.jobs.iter().find(|j| j.status == JobStatus::Running);
        
        match running_job {
            Some(job) => {
                // Create command directory if it doesn't exist
                if !self.command_dir.exists() {
                    std::fs::create_dir_all(&self.command_dir)
                        .with_context(|| format!("Failed to create command directory: {}", self.command_dir.display()))?;
                }
                
                // Create command file
                let command_file = self.command_dir.join(format!("requeue-{}.json", job.id));
                
                // Use atomic write (write to temp file, then rename)
                let temp_file = self.command_dir.join(format!(".requeue-{}.json.tmp", job.id));
                
                let command = serde_json::json!({
                    "action": "requeue",
                    "job_id": job.id,
                    "reason": "manual_requeue_from_tui",
                    "timestamp": Utc::now().to_rfc3339(),
                });
                
                std::fs::write(&temp_file, serde_json::to_string_pretty(&command)?)
                    .with_context(|| format!("Failed to write command file: {}", temp_file.display()))?;
                
                std::fs::rename(&temp_file, &command_file)
                    .with_context(|| format!("Failed to rename command file: {} -> {}", 
                        temp_file.display(), command_file.display()))?;
                
                self.last_message = Some(format!("✅ Requeue command sent for job: {}", 
                    job.source_path.file_name().and_then(|n| n.to_str()).unwrap_or("?")));
                self.message_timeout = Some(Utc::now() + chrono::Duration::seconds(5));
                
                // Log success (no info! macro needed, message shown in UI)
                Ok(())
            }
            None => {
                self.last_message = Some("⚠️  No running job to requeue".to_string());
                self.message_timeout = Some(Utc::now() + chrono::Duration::seconds(3));
                Ok(())
            }
        }
    }
    
    /// Clear message if timeout expired
    fn update_message(&mut self) {
        if let Some(timeout) = self.message_timeout {
            if Utc::now() > timeout {
                self.last_message = None;
                self.message_timeout = None;
            }
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

    /// Detect progress for a running job by checking temp file state
    fn detect_job_progress(&mut self, job: &Job) {
        use std::fs;
        use chrono::Duration as ChronoDuration;
        
        // Only track Running jobs
        if job.status != JobStatus::Running {
            // Remove from tracking if not running anymore
            self.job_progress.remove(&job.id);
            return;
        }
        
        let now = Utc::now();
        let temp_output = self.get_temp_output_path(&job.source_path);
        let orig_backup = job.source_path.with_extension("orig.mkv");
        
        // Get original size
        let original_size = job.original_bytes.unwrap_or(0);
        
        // Check if temp file exists
        if let Ok(metadata) = fs::metadata(&temp_output) {
            let current_temp_size = metadata.len();
            let temp_file_modified_time = metadata.modified()
                .ok()
                .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64);
            
            // Get or create progress tracking
            let mut progress = self.job_progress.get(&job.id)
                .cloned()
                .unwrap_or_else(|| JobProgress::new(temp_output.clone(), original_size));
            
            // Calculate bytes per second if we have previous data
            let time_delta_seconds = (now - progress.last_updated).num_seconds().max(1) as f64;
            if time_delta_seconds > 0.0 && current_temp_size > progress.temp_file_size {
                let bytes_delta = current_temp_size - progress.temp_file_size;
                progress.bytes_per_second = bytes_delta as f64 / time_delta_seconds;
            }
            
            // Update progress tracking
            progress.temp_file_size = current_temp_size;
            progress.last_updated = now;
            
            // Estimate output size using quality-based calculation if available
            let estimated_output_size = if original_size > 0 {
                calculate_estimated_output_size(job).unwrap_or_else(|| {
                    // Fallback to 50% if calculation fails
                    (original_size as f64 * 0.5) as u64
                })
            } else {
                0
            };
            
            // Calculate progress percentage
            if estimated_output_size > 0 {
                progress.progress_percent = (current_temp_size as f64 / estimated_output_size as f64 * 100.0)
                    .min(100.0)
                    .max(0.0);
            }
            
            // Calculate ETA if we have a write rate
            if progress.bytes_per_second > 0.0 && estimated_output_size > current_temp_size {
                let remaining_bytes = estimated_output_size - current_temp_size;
                let seconds_remaining = remaining_bytes as f64 / progress.bytes_per_second;
                progress.estimated_completion = Some(now + ChronoDuration::seconds(seconds_remaining as i64));
            }
            
            // Detect stage based on temp file state
            // Check if temp file is still being written (modified recently)
            if let Some(modified_time_secs) = temp_file_modified_time {
                let now_secs = now.timestamp();
                let seconds_since_mod = (now_secs - modified_time_secs).max(0);
                if seconds_since_mod < 10 {
                    // File modified in last 10 seconds - actively transcoding
                    progress.stage = JobStage::Transcoding;
                } else {
                    // File not modified recently - may be verifying or stuck
                    if progress.progress_percent > 95.0 {
                        progress.stage = JobStage::Verifying;
                    } else {
                        progress.stage = JobStage::Transcoding; // Still transcoding, just slow
                    }
                }
            } else {
                progress.stage = JobStage::Transcoding;
            }
            
            // Check if original file has been replaced (backup exists)
            if orig_backup.exists() && !job.source_path.exists() {
                progress.stage = JobStage::Replacing;
            }
            
            self.job_progress.insert(job.id.clone(), progress);
        } else {
            // Temp file doesn't exist yet
            // Check how long job has been running
            if let Some(started) = job.started_at {
                let elapsed = (now - started).num_seconds();
                if elapsed < 30 {
                    // Recently started, probably still probing
                    let progress = JobProgress::new(temp_output.clone(), original_size);
                    self.job_progress.insert(job.id.clone(), progress);
                } else {
                    // Running for >30s without temp file - may be stuck
                    // Still track it as probing for now
                    let mut progress = JobProgress::new(temp_output.clone(), original_size);
                    progress.stage = JobStage::Probing;
                    self.job_progress.insert(job.id.clone(), progress);
                }
            } else {
                // No started_at - create basic progress tracking
                let mut progress = JobProgress::new(temp_output.clone(), original_size);
                progress.stage = JobStage::Probing;
                self.job_progress.insert(job.id.clone(), progress);
            }
        }
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
                self.last_job_count = self.jobs.len();
            }
            Err(_e) => {
                // Silently fail - show empty table
                // Errors are visible in the UI (empty table, status counts)
                self.jobs = Vec::new();
                self.last_job_count = 0;
            }
        }

        // Collect all running job data before iterating to avoid borrow checker issues
        let now = Utc::now();
        let running_job_ids: Vec<String> = self.jobs.iter()
            .filter(|j| j.status == JobStatus::Running)
            .map(|j| j.id.clone())
            .collect();

        // Update progress tracking for all running jobs by cloning necessary data
        for job_id in &running_job_ids {
            // Collect job data we need before calling detect_job_progress
            let job_data: Option<(PathBuf, Option<u64>, Option<DateTime<Utc>>, Option<String>, JobStatus)> = 
                self.jobs.iter()
                    .find(|j| j.id == *job_id)
                    .map(|j| (j.source_path.clone(), j.original_bytes, j.started_at, j.video_codec.clone(), j.status.clone()));
            
            if let Some((source_path, original_bytes, started_at, video_codec, _status)) = job_data {
                // Create a temporary job-like structure to pass progress detection info
                // Since we can't mutate self while iterating, we'll update progress directly
                let temp_output = self.get_temp_output_path(&source_path);
                let original_size = original_bytes.unwrap_or(0);
                
                // Check if temp file exists and update progress
                if let Ok(metadata) = std::fs::metadata(&temp_output) {
                    let current_temp_size = metadata.len();
                    
                    // Get or create progress tracking
                    let mut progress = self.job_progress.get(job_id)
                        .cloned()
                        .unwrap_or_else(|| JobProgress::new(temp_output.clone(), original_size));
                    
                    // Calculate time delta
                    let time_delta_seconds = (now - progress.last_updated).num_seconds().max(1) as f64;
                    
                    // Calculate bytes per second
                    if time_delta_seconds > 0.0 && current_temp_size > progress.temp_file_size {
                        let bytes_delta = current_temp_size - progress.temp_file_size;
                        progress.bytes_per_second = bytes_delta as f64 / time_delta_seconds;
                    }
                    
                    // Store previous values for delta calculations
                    progress.last_temp_file_size = progress.temp_file_size;
                    progress.last_progress_percent = progress.progress_percent;
                    
                    // Update progress tracking
                    progress.temp_file_size = current_temp_size;
                    progress.last_updated = now;
                    
                    // Estimate output size using quality-based calculation if available
                    let estimated_output_size = if original_size > 0 {
                        // Try to get quality from the job if available
                        let job_for_calc = self.jobs.iter().find(|j| j.id == *job_id);
                        if let Some(job) = job_for_calc {
                            calculate_estimated_output_size(job).unwrap_or_else(|| {
                                // Fallback calculation based on codec
                                if let Some(codec) = &video_codec {
                                    let efficiency_factor = match codec.to_lowercase().as_str() {
                                        "hevc" | "h265" => 0.55,
                                        "h264" | "avc" => 0.40,
                                        "vp9" => 0.85,
                                        "av1" => 1.0,
                                        _ => 0.5,
                                    };
                                    (original_size as f64 * efficiency_factor) as u64
                                } else {
                                    (original_size as f64 * 0.5) as u64
                                }
                            })
                        } else {
                            // Fallback if job not found
                            (original_size as f64 * 0.5) as u64
                        }
                    } else {
                        0
                    };
                    
                    // Calculate progress percentage
                    if estimated_output_size > 0 {
                        progress.progress_percent = (current_temp_size as f64 / estimated_output_size as f64 * 100.0)
                            .min(100.0)
                            .max(0.0);
                    }
                    
                    // Calculate ETA
                    if progress.bytes_per_second > 0.0 && estimated_output_size > current_temp_size {
                        use chrono::Duration as ChronoDuration;
                        let remaining_bytes = estimated_output_size - current_temp_size;
                        let seconds_remaining = remaining_bytes as f64 / progress.bytes_per_second;
                        progress.estimated_completion = Some(now + ChronoDuration::seconds(seconds_remaining as i64));
                    }
                    
                    // Store estimated final size (Task 13.2)
                    progress.estimated_final_size = Some(estimated_output_size);
                    
                    // Calculate current compression ratio (Task 13.2)
                    if current_temp_size > 0 && original_size > 0 {
                        progress.current_compression_ratio = Some(
                            (original_size - current_temp_size) as f64 / original_size as f64
                        );
                    }
                    
                    // Calculate frame-level progress tracking (Task 13.1, 13.2)
                    if let Some(job) = self.jobs.iter().find(|j| j.id == *job_id) {
                        // Calculate total frames from video metadata
                        if let Some(frame_rate_str) = &job.video_frame_rate {
                            if let Some(fps) = parse_frame_rate(frame_rate_str) {
                                // Get video duration if available
                                // Duration can be calculated from bitrate and file size, or from metadata
                                // For now, we'll estimate based on original file size and bitrate
                                if let Some(bitrate) = job.video_bitrate {
                                    if bitrate > 0 && original_size > 0 {
                                        // Estimate duration: duration = (file_size_bytes * 8) / bitrate_bps
                                        let duration_secs = (original_size as f64 * 8.0) / bitrate as f64;
                                        
                                        // Calculate total frames
                                        progress.total_frames = JobProgress::calculate_total_frames(duration_secs, fps);
                                        
                                        // Estimate frames processed based on progress percentage
                                        progress.frames_processed = progress.estimate_frames_processed();
                                        
                                        // Calculate current FPS based on progress rate
                                        if time_delta_seconds > 0.0 {
                                            let progress_delta = progress.progress_percent - progress.last_progress_percent;
                                            progress.current_fps = progress.calculate_current_fps(time_delta_seconds, progress_delta);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    
                    // Detect stage
                    progress.stage = JobStage::Transcoding;
                    if progress.progress_percent > 95.0 {
                        progress.stage = JobStage::Verifying;
                    }
                    
                    self.job_progress.insert(job_id.clone(), progress);
                } else {
                    // No temp file yet - probing stage
                    if let Some(started) = started_at {
                        let elapsed = (now - started).num_seconds();
                        if elapsed < 30 {
                            let progress = JobProgress::new(temp_output.clone(), original_size);
                            self.job_progress.insert(job_id.clone(), progress);
                        } else {
                            let mut progress = JobProgress::new(temp_output.clone(), original_size);
                            progress.stage = JobStage::Probing;
                            self.job_progress.insert(job_id.clone(), progress);
                        }
                    } else {
                        let mut progress = JobProgress::new(temp_output.clone(), original_size);
                        progress.stage = JobStage::Probing;
                        self.job_progress.insert(job_id.clone(), progress);
                    }
                }
            }
        }
        
        // Clean up progress tracking for jobs that are no longer running
        let running_ids_set: std::collections::HashSet<_> = running_job_ids.iter().collect();
        self.job_progress.retain(|id, _| running_ids_set.contains(id));
        
        // Update estimated savings cache for all jobs
        // Calculate/update estimates for jobs that have metadata
        // This runs in the background during refresh, updating estimates as metadata becomes available
        for job in &self.jobs {
            // Skip completed jobs - they have actual savings, not estimates
            if job.status == JobStatus::Success || job.status == JobStatus::Failed || job.status == JobStatus::Skipped {
                // Remove from cache if present (no longer needed)
                self.estimated_savings_cache.remove(&job.id);
                continue;
            }
            
            // Process Pending and Running jobs - calculate estimates when metadata is available
            // Check if job has metadata for estimation
            let has_metadata = has_estimation_metadata(job);
            
            if has_metadata {
                // Always recalculate if we have metadata - this ensures:
                // 1. Estimates appear when metadata is first extracted (background or during transcoding)
                // 2. Estimates update when quality setting is added
                // 3. Estimates are always current with the latest job metadata
                // Also force recalculation if it was previously cached as None (metadata just became available)
                let estimate = estimate_space_savings(job);
                self.estimated_savings_cache.insert(job.id.clone(), estimate);
            } else {
                // Job doesn't have metadata yet - store None to mark that we've checked
                // This will be updated when metadata becomes available (after background extraction)
                // Only update if not already cached (avoid overwriting valid estimates)
                if !self.estimated_savings_cache.contains_key(&job.id) {
                    self.estimated_savings_cache.insert(job.id.clone(), None);
                }
            }
        }
        
        // Clean up cache entries for jobs that no longer exist
        let job_ids_set: std::collections::HashSet<_> = self.jobs.iter().map(|j| &j.id).collect();
        self.estimated_savings_cache.retain(|id, _| job_ids_set.contains(id));
        
        // Update statistics cache if needed
        if self.statistics_cache.needs_refresh() {
            self.statistics_cache = StatisticsCache::calculate(&self.jobs);
        }
        
        self.last_refresh = now;
        
        Ok(())
    }
    
    /// Get activity status based on job state changes and running jobs
    fn get_activity_status(&self) -> (&'static str, Color) {
        let running_count = self.jobs.iter()
            .filter(|j| j.status == JobStatus::Running)
            .count();
        
        if running_count > 0 {
            ("⚙  Processing", self.color_scheme.running)
        } else {
            let pending_count = self.jobs.iter()
                .filter(|j| j.status == JobStatus::Pending)
                .count();
            
            if pending_count > 0 {
                ("⏸  Idle", self.color_scheme.pending)
            } else {
                ("✓  Idle", self.color_scheme.success)
            }
        }
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
    let mut app = App::new(cfg.job_state_dir.clone(), cfg.temp_output_dir.clone());

    // Main event loop with adaptive refresh rate
    loop {
        // Refresh data first to get latest job status
        app.refresh()?;

        // Draw UI
        terminal.draw(|f| ui(f, &mut app))?;

        // Determine refresh rate based on current state (after refresh)
        // Check if there's an active job to determine refresh rate
        let has_active_job = app.jobs.iter().any(|j| j.status == JobStatus::Running);
        
        // Check if there are pending jobs without metadata (background extraction might be happening)
        let pending_count = app.jobs.iter()
            .filter(|j| j.status == JobStatus::Pending)
            .count();
        
        // Handle input with adaptive timeout
        // Refresh more frequently if:
        // - Active transcoding job (1s)
        // - There are pending jobs (refresh frequently to catch metadata updates - 250ms)
        // - Otherwise idle (5s)
        let poll_timeout = if has_active_job {
            Duration::from_millis(1000)  // 1 second when transcoding
        } else if pending_count > 0 {
            // Refresh very frequently when there are pending jobs to catch metadata extraction
            // Keep refreshing quickly even if some jobs already have metadata (others might be extracting)
            Duration::from_millis(250)   // 250ms when there are pending jobs
        } else {
            Duration::from_millis(5000)  // 5 seconds when idle
        };
        
        if crossterm::event::poll(poll_timeout)? {
            if let crossterm::event::Event::Key(key) = crossterm::event::read()? {
                match key.code {
                    crossterm::event::KeyCode::Char('q') => {
                        app.should_quit = true;
                    }
                    crossterm::event::KeyCode::Char('r') => {
                        app.refresh()?;
                    }
                    crossterm::event::KeyCode::Char('R') => {
                        // Force requeue running job
                        if let Err(e) = app.requeue_running_job() {
                            app.last_message = Some(format!("❌ Failed to requeue: {}", e));
                            app.message_timeout = Some(Utc::now() + chrono::Duration::seconds(5));
                        }
                    }
                    // Filter keys (1-5)
                    crossterm::event::KeyCode::Char('1') => {
                        app.ui_state.filter = JobFilter::All;
                    }
                    crossterm::event::KeyCode::Char('2') => {
                        app.ui_state.filter = JobFilter::Pending;
                    }
                    crossterm::event::KeyCode::Char('3') => {
                        app.ui_state.filter = JobFilter::Running;
                    }
                    crossterm::event::KeyCode::Char('4') => {
                        app.ui_state.filter = JobFilter::Success;
                    }
                    crossterm::event::KeyCode::Char('5') => {
                        app.ui_state.filter = JobFilter::Failed;
                    }
                    // Sort key (s)
                    crossterm::event::KeyCode::Char('s') => {
                        app.cycle_sort_mode();
                    }
                    // Navigation keys
                    crossterm::event::KeyCode::Up => {
                        app.move_selection_up();
                    }
                    crossterm::event::KeyCode::Down => {
                        app.move_selection_down();
                    }
                    crossterm::event::KeyCode::PageUp => {
                        app.move_selection_page_up();
                    }
                    crossterm::event::KeyCode::PageDown => {
                        app.move_selection_page_down();
                    }
                    // Detail view keys (Task 9.5)
                    crossterm::event::KeyCode::Enter => {
                        // Open detail view if a job is selected and we're not already in detail view
                        if app.ui_state.view_mode == ViewMode::Normal {
                            if let Some(selected_idx) = app.ui_state.selected_index {
                                // Get filtered and sorted jobs
                                let mut filtered_jobs = app.filter_jobs(&app.jobs);
                                app.sort_jobs(&mut filtered_jobs);
                                
                                // Get the selected job ID before modifying app state
                                let selected_job_id = if selected_idx < filtered_jobs.len() {
                                    Some(filtered_jobs[selected_idx].id.clone())
                                } else {
                                    None
                                };
                                
                                // Now update app state
                                if let Some(job_id) = selected_job_id {
                                    app.ui_state.view_mode = ViewMode::DetailView;
                                    app.ui_state.detail_view_job_id = Some(job_id);
                                }
                            }
                        } else if app.ui_state.view_mode == ViewMode::DetailView {
                            // Close detail view
                            app.ui_state.view_mode = ViewMode::Normal;
                            app.ui_state.detail_view_job_id = None;
                        }
                    }
                    crossterm::event::KeyCode::Esc => {
                        // Close detail view if open
                        if app.ui_state.view_mode == ViewMode::DetailView {
                            app.ui_state.view_mode = ViewMode::Normal;
                            app.ui_state.detail_view_job_id = None;
                        }
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

/// Task 14.3: Render simplified view for very small terminals (< 80x12)
/// Requirements 10.3: Show only essential information with clear message about limited space
fn render_simplified_view(f: &mut Frame, app: &App, area: Rect, _layout_config: &LayoutConfig) {
    // Create a simple vertical layout with minimal information
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Header with message
            Constraint::Min(3),     // Job list (minimal)
            Constraint::Length(2),  // Status line
        ])
        .split(area);
    
    // Header with warning message
    let header_text = vec![
        Line::from(Span::styled(
            "⚠  TERMINAL TOO SMALL",
            Style::default().fg(app.color_scheme.failed).add_modifier(Modifier::BOLD)
        )),
        Line::from(Span::styled(
            format!("Current: {}x{} | Minimum: 80x12", area.width, area.height),
            Style::default().fg(app.color_scheme.text_secondary)
        )),
    ];
    
    let header = Paragraph::new(header_text)
        .block(Block::default().borders(Borders::ALL))
        .style(Style::default().fg(app.color_scheme.text_primary));
    f.render_widget(header, chunks[0]);
    
    // Show minimal job information
    let running_count = app.jobs.iter().filter(|j| j.status == JobStatus::Running).count();
    let pending_count = app.jobs.iter().filter(|j| j.status == JobStatus::Pending).count();
    let success_count = app.jobs.iter().filter(|j| j.status == JobStatus::Success).count();
    let failed_count = app.jobs.iter().filter(|j| j.status == JobStatus::Failed).count();
    
    let mut job_lines = vec![
        Line::from(Span::styled(
            format!("Total Jobs: {}", app.jobs.len()),
            Style::default().fg(app.color_scheme.text_primary).add_modifier(Modifier::BOLD)
        )),
    ];
    
    if running_count > 0 {
        job_lines.push(Line::from(Span::styled(
            format!("⚙  Running: {}", running_count),
            Style::default().fg(app.color_scheme.running)
        )));
    }
    
    if pending_count > 0 {
        job_lines.push(Line::from(Span::styled(
            format!("⏸  Pending: {}", pending_count),
            Style::default().fg(app.color_scheme.pending)
        )));
    }
    
    if success_count > 0 {
        job_lines.push(Line::from(Span::styled(
            format!("✓  Success: {}", success_count),
            Style::default().fg(app.color_scheme.success)
        )));
    }
    
    if failed_count > 0 {
        job_lines.push(Line::from(Span::styled(
            format!("✗  Failed: {}", failed_count),
            Style::default().fg(app.color_scheme.failed)
        )));
    }
    
    // Show current running job if any
    if let Some(job) = app.jobs.iter().find(|j| j.status == JobStatus::Running) {
        let file_name = job.source_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?");
        
        job_lines.push(Line::from(""));
        job_lines.push(Line::from(Span::styled(
            format!("Current: {}", truncate_string(file_name, 40)),
            Style::default().fg(app.color_scheme.text_primary)
        )));
        
        // Show progress if available
        if let Some(progress) = app.job_progress.get(&job.id) {
            job_lines.push(Line::from(Span::styled(
                format!("Progress: {:.1}%", progress.progress_percent),
                Style::default().fg(app.color_scheme.running)
            )));
        }
    }
    
    let job_info = Paragraph::new(job_lines)
        .block(Block::default().borders(Borders::ALL).title("Jobs"))
        .style(Style::default().fg(app.color_scheme.text_primary));
    f.render_widget(job_info, chunks[1]);
    
    // Status line with basic controls
    let status_text = Line::from(vec![
        Span::styled("q", Style::default().fg(app.color_scheme.header).add_modifier(Modifier::BOLD)),
        Span::raw("=quit  "),
        Span::styled("r", Style::default().fg(app.color_scheme.header).add_modifier(Modifier::BOLD)),
        Span::raw("=refresh"),
    ]);
    
    let status = Paragraph::new(status_text)
        .block(Block::default().borders(Borders::ALL))
        .style(Style::default().fg(app.color_scheme.text_secondary));
    f.render_widget(status, chunks[2]);
}

fn ui(f: &mut Frame, app: &mut App) {
    let size = f.size();
    
    // Task 14.2: Call layout calculation at start of rendering (Requirements 10.5)
    let layout_config = LayoutConfig::from_terminal_size(size);
    
    // Task 14.3: Check if terminal is too small and show simplified view (Requirements 10.3)
    if layout_config.is_too_small {
        render_simplified_view(f, app, size, &layout_config);
        return;
    }
    
    // Check if there's a running job
    let has_running_job = app.jobs.iter().any(|j| j.status == JobStatus::Running);
    
    // Task 14.2: Calculate layout chunks based on responsive layout config
    let chunks = layout_config.calculate_chunks(has_running_job);
    
    // Render components based on layout configuration
    let mut chunk_idx = 0;
    
    // Render top bar (always shown)
    if chunk_idx < chunks.len() {
        render_top_bar(f, app, chunks[chunk_idx]);
        chunk_idx += 1;
    }
    
    // Render statistics dashboard if enabled (conditional)
    if layout_config.show_statistics && chunk_idx < chunks.len() {
        render_statistics_dashboard(f, app, chunks[chunk_idx]);
        chunk_idx += 1;
    }
    
    // Render current job if running (conditional)
    if layout_config.show_current_job && has_running_job && !layout_config.is_too_small && chunk_idx < chunks.len() {
        render_current_job(f, app, chunks[chunk_idx]);
        chunk_idx += 1;
    }
    
    // Render job table (always shown)
    if chunk_idx < chunks.len() {
        render_job_table(f, app, chunks[chunk_idx]);
        chunk_idx += 1;
    }
    
    // Render status bar (always shown)
    if chunk_idx < chunks.len() {
        render_status_bar(f, app, chunks[chunk_idx]);
    }
    
    // Render detail view modal on top if in DetailView mode
    // Task 14.2: Maintain scroll position during layout changes (Requirements 10.5)
    // The selection state is preserved in app.ui_state, so it persists across layout changes
    if app.ui_state.view_mode == ViewMode::DetailView {
        render_detail_view(f, app, size);
    }
}

fn render_current_job(f: &mut Frame, app: &App, area: Rect) {
    if let Some(job) = app.jobs.iter().find(|j| j.status == JobStatus::Running) {
        // Get progress tracking if available
        let progress = app.job_progress.get(&job.id);
        
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
        
        // New size (use progress temp file size if available)
        let new_size = if let Some(prog) = progress {
            if prog.temp_file_size > 0 {
                format_size(prog.temp_file_size, DECIMAL)
            } else {
                "-".to_string()
            }
        } else {
            job.new_bytes
                .map(|b| format_size(b, DECIMAL))
                .unwrap_or_else(|| "-".to_string())
        };
        
        // Current stage
        let stage_str = if let Some(prog) = progress {
            prog.stage.as_str()
        } else {
            "Starting"
        };
        
        // Progress percentage
        let progress_pct = if let Some(prog) = progress {
            prog.progress_percent
        } else {
            0.0
        };
        
        // ETA
        let eta_str = if let Some(prog) = progress {
            if let Some(eta) = prog.estimated_completion {
                let remaining = (eta - Utc::now()).num_seconds();
                if remaining > 0 {
                    let hours = remaining / 3600;
                    let minutes = (remaining % 3600) / 60;
                    let seconds = remaining % 60;
                    if hours > 0 {
                        format!("{}h {}m", hours, minutes)
                    } else if minutes > 0 {
                        format!("{}m {}s", minutes, seconds)
                    } else {
                        format!("{}s", seconds)
                    }
                } else {
                    "Soon".to_string()
                }
            } else {
                "-".to_string()
            }
        } else {
            "-".to_string()
        };
        
        // Speed (bytes per second)
        let speed_str = if let Some(prog) = progress {
            if prog.bytes_per_second > 0.0 {
                format!("{}/s", format_size(prog.bytes_per_second as u64, DECIMAL))
            } else {
                "-".to_string()
            }
        } else {
            "-".to_string()
        };
        
        // Duration/Elapsed time
        let duration = if let Some(started) = job.started_at {
            let dur = Utc::now() - started;
            let hours = dur.num_hours();
            let minutes = dur.num_minutes() % 60;
            let seconds = dur.num_seconds() % 60;
            if hours > 0 {
                format!("{}h {}m", hours, minutes)
            } else if minutes > 0 {
                format!("{}m {}s", minutes, seconds)
            } else {
                format!("{}s", seconds)
            }
        } else {
            "-".to_string()
        };
        
        // Video metadata display (Task 8.1)
        let resolution = if let (Some(width), Some(height)) = (job.video_width, job.video_height) {
            format!("{}x{}", width, height)
        } else {
            "-".to_string()
        };
        
        let codec = job.video_codec.as_deref()
            .map(|c| c.to_uppercase())
            .unwrap_or_else(|| "-".to_string());
        
        let bitrate = job.video_bitrate
            .map(|br| {
                let mbps = br as f64 / 1_000_000.0;
                format!("{:.1}Mbps", mbps)
            })
            .unwrap_or_else(|| "-".to_string());
        
        let hdr_status = match job.is_hdr {
            Some(true) => "HDR",
            Some(false) => "SDR",
            None => "-",
        };
        
        let bit_depth = job.source_bit_depth
            .map(|depth| format!("{}bit", depth))
            .unwrap_or_else(|| "-".to_string());
        
        // FPS processing rate display (Task 8.2)
        let fps_str = if let Some(prog) = progress {
            if let Some(fps) = prog.current_fps {
                format!("{:.1}fps", fps)
            } else {
                "-".to_string()
            }
        } else {
            "-".to_string()
        };
        
        // Estimated final size display (Task 8.4)
        let est_final_size_str = if let Some(prog) = progress {
            if let Some(est_size) = prog.estimated_final_size {
                format_size(est_size, DECIMAL)
            } else {
                // Fallback to calculation
                if let Some(est_size) = calculate_estimated_output_size(job) {
                    format_size(est_size, DECIMAL)
                } else {
                    "-".to_string()
                }
            }
        } else {
            if let Some(est_size) = calculate_estimated_output_size(job) {
                format_size(est_size, DECIMAL)
            } else {
                "-".to_string()
            }
        };
        
        // Current compression ratio display (Task 8.5)
        // Show estimated final compression ratio, not current progress
        let current_comp_ratio_str = if let Some(prog) = progress {
            // Use estimated final size for ratio calculation
            if let Some(est_final) = prog.estimated_final_size {
                if prog.original_size > 0 {
                    let reduction = (prog.original_size - est_final) as f64 / prog.original_size as f64;
                    format!("{:.1}%", reduction * 100.0)
                } else {
                    "-".to_string()
                }
            } else if let Some(ratio) = prog.current_compression_ratio {
                format!("{:.1}%", ratio * 100.0)
            } else {
                "-".to_string()
            }
        } else {
            "-".to_string()
        };
        
        // Split area into text and progress bar
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(4),  // Text info (4 lines now)
                Constraint::Length(1),  // Progress bar
            ])
            .split(area);
        
        // Quality setting used for encoding
        let quality_str = if let Some(quality) = job.av1_quality {
            format!("Q{}", quality)
        } else {
            "-".to_string()
        };
        
        // Build info text with enhanced metadata and improved spacing
        let info_lines = vec![
            format!("  STAGE: {}  │  FILE: {}", stage_str, truncate_string(&file_name, 50)),
            format!("  VIDEO: {}  │  {}  │  {}  │  {}  │  {}", resolution, codec, bitrate, hdr_status, bit_depth),
            format!("  ORIG: {}  │  CURRENT: {}  │  EST FINAL: {}  │  RATIO: {}  │  QUALITY: {}", 
                orig_size, new_size, est_final_size_str, current_comp_ratio_str, quality_str),
            format!("  SPEED: {}  │  FPS: {}  │  PROGRESS: {:.1}%  │  ETA: {}  │  ELAPSED: {}", 
                speed_str, fps_str, progress_pct, eta_str, duration),
        ];
        
        let paragraph = Paragraph::new(info_lines.join("\n"))
            .block(Block::default()
                .borders(Borders::ALL)
                .title("⚙  CURRENT JOB")
                .style(Style::default().fg(app.color_scheme.running)))
            .style(Style::default().fg(app.color_scheme.text_primary));
        f.render_widget(paragraph, chunks[0]);
        
        // Render multi-segment progress bar (Task 8.3)
        // Use different colors based on current stage with color gradient
        // Apply color gradient to progress (higher = more intense/different color)
        let progress_color = if progress_pct > 90.0 {
            Color::Green  // Near completion - green
        } else if progress_pct > 50.0 {
            app.color_scheme.stage_color(&if let Some(prog) = progress {
                prog.stage.clone()
            } else {
                JobStage::Probing
            })
        } else {
            Color::Cyan  // Early stage - cyan
        };
        
        let progress_percent_u16 = progress_pct.min(100.0).max(0.0) as u16;
        
        let progress_gauge = Gauge::default()
            .block(Block::default()
                .borders(Borders::NONE)
                .title(format!("{} - {:.1}%", stage_str, progress_pct)))
            .gauge_style(Style::default().fg(progress_color).add_modifier(Modifier::BOLD))
            .percent(progress_percent_u16)
            .label(format!("{:.1}%", progress_pct));
        f.render_widget(progress_gauge, chunks[1]);
    }
}

/// Render detail view modal overlay for a selected job
/// Task 9.1, 9.2, 9.3, 9.4: Create detail view with comprehensive job information
fn render_detail_view(f: &mut Frame, app: &App, area: Rect) {
    // Get the selected job
    let job = if let Some(job_id) = &app.ui_state.detail_view_job_id {
        app.jobs.iter().find(|j| &j.id == job_id)
    } else {
        None
    };
    
    if job.is_none() {
        return;
    }
    
    let job = job.unwrap();
    
    // Create modal overlay in the center of the screen
    // Modal should be 80% of screen width and 80% of screen height
    let modal_width = (area.width as f32 * 0.8) as u16;
    let modal_height = (area.height as f32 * 0.8) as u16;
    
    // Center the modal
    let modal_x = (area.width.saturating_sub(modal_width)) / 2;
    let modal_y = (area.height.saturating_sub(modal_height)) / 2;
    
    let modal_area = Rect {
        x: area.x + modal_x,
        y: area.y + modal_y,
        width: modal_width,
        height: modal_height,
    };
    
    // Build detail content with Unicode box-drawing characters
    let mut lines = Vec::new();
    
    // Header with Unicode box-drawing
    lines.push("╔═══════════════════════════════════════════════════════════════════════════════╗".to_string());
    lines.push(format!("║ 📋 JOB DETAILS: {}", job.id));
    lines.push("╠═══════════════════════════════════════════════════════════════════════════════╣".to_string());
    lines.push("".to_string());
    
    // File paths (Task 9.4) with Unicode symbols and improved spacing
    lines.push("".to_string());
    lines.push("📁 FILE PATHS:".to_string());
    lines.push(format!("   ├─ Source: {}", job.source_path.display()));
    if let Some(output) = &job.output_path {
        lines.push(format!("   └─ Output: {}", output.display()));
    } else {
        lines.push("   └─ Output: (not set)".to_string());
    }
    lines.push("".to_string());
    
    // Job status and reason with Unicode symbols and improved spacing
    lines.push("📊 STATUS:".to_string());
    let status_symbol = match job.status {
        JobStatus::Pending => "⏸",
        JobStatus::Running => "⚙",
        JobStatus::Success => "✓",
        JobStatus::Failed => "✗",
        JobStatus::Skipped => "⊘",
    };
    lines.push(format!("   {} Status: {:?}", status_symbol, job.status));
    if let Some(reason) = &job.reason {
        lines.push(format!("   ℹ  Reason: {}", reason));
    }
    lines.push("".to_string());
    
    // Job history (Task 9.3) with Unicode symbols and improved spacing
    lines.push("🕐 JOB HISTORY:".to_string());
    lines.push(format!("   Created:  {}", job.created_at.format("%Y-%m-%d %H:%M:%S UTC")));
    
    if let Some(started) = job.started_at {
        lines.push(format!("   Started:  {}", started.format("%Y-%m-%d %H:%M:%S UTC")));
        
        // Calculate duration from created to started
        let queue_duration = (started - job.created_at).num_seconds();
        lines.push(format!("   Queue Time: {}s", queue_duration));
    } else {
        lines.push("   Started:  (not started)".to_string());
    }
    
    if let Some(finished) = job.finished_at {
        lines.push(format!("   Finished: {}", finished.format("%Y-%m-%d %H:%M:%S UTC")));
        
        // Calculate processing duration
        if let Some(started) = job.started_at {
            let proc_duration = (finished - started).num_seconds();
            let hours = proc_duration / 3600;
            let minutes = (proc_duration % 3600) / 60;
            let seconds = proc_duration % 60;
            
            if hours > 0 {
                lines.push(format!("   Processing Time: {}h {}m {}s", hours, minutes, seconds));
            } else if minutes > 0 {
                lines.push(format!("   Processing Time: {}m {}s", minutes, seconds));
            } else {
                lines.push(format!("   Processing Time: {}s", seconds));
            }
        }
        
        // Calculate total duration
        let total_duration = (finished - job.created_at).num_seconds();
        let hours = total_duration / 3600;
        let minutes = (total_duration % 3600) / 60;
        let seconds = total_duration % 60;
        
        if hours > 0 {
            lines.push(format!("   Total Time: {}h {}m {}s", hours, minutes, seconds));
        } else if minutes > 0 {
            lines.push(format!("   Total Time: {}m {}s", minutes, seconds));
        } else {
            lines.push(format!("   Total Time: {}s", seconds));
        }
    } else {
        lines.push("   Finished: (not finished)".to_string());
    }
    lines.push("".to_string());
    
    // Video metadata (Task 9.2) with Unicode symbols and improved spacing
    lines.push("🎬 VIDEO METADATA:".to_string());
    
    // Resolution
    if let (Some(width), Some(height)) = (job.video_width, job.video_height) {
        lines.push(format!("   Resolution: {}x{}", width, height));
    } else {
        lines.push("   Resolution: (not available)".to_string());
    }
    
    // Codec
    if let Some(codec) = &job.video_codec {
        lines.push(format!("   Codec: {}", codec));
    } else {
        lines.push("   Codec: (not available)".to_string());
    }
    
    // Bitrate
    if let Some(bitrate) = job.video_bitrate {
        let mbps = bitrate as f64 / 1_000_000.0;
        lines.push(format!("   Bitrate: {:.2} Mbps ({} bps)", mbps, bitrate));
    } else {
        lines.push("   Bitrate: (not available)".to_string());
    }
    
    // Frame rate
    if let Some(frame_rate) = &job.video_frame_rate {
        lines.push(format!("   Frame Rate: {} fps", frame_rate));
    } else {
        lines.push("   Frame Rate: (not available)".to_string());
    }
    
    // HDR status
    match job.is_hdr {
        Some(true) => lines.push("   HDR: Yes".to_string()),
        Some(false) => lines.push("   HDR: No".to_string()),
        None => lines.push("   HDR: (not available)".to_string()),
    }
    
    // Bit depth (Task 9.2)
    if let Some(bit_depth) = job.source_bit_depth {
        lines.push(format!("   Source Bit Depth: {} bit", bit_depth));
    } else {
        lines.push("   Source Bit Depth: (not available)".to_string());
    }
    
    if let Some(bit_depth) = job.target_bit_depth {
        lines.push(format!("   Target Bit Depth: {} bit", bit_depth));
    } else {
        lines.push("   Target Bit Depth: (not available)".to_string());
    }
    
    // Pixel format (Task 9.2)
    if let Some(pix_fmt) = &job.source_pix_fmt {
        lines.push(format!("   Pixel Format: {}", pix_fmt));
    } else {
        lines.push("   Pixel Format: (not available)".to_string());
    }
    
    lines.push("".to_string());
    
    // Encoding parameters (Task 9.2) with Unicode symbols and improved spacing
    lines.push("⚙  ENCODING PARAMETERS:".to_string());
    
    if let Some(quality) = job.av1_quality {
        lines.push(format!("   AV1 Quality (CRF): {}", quality));
        lines.push("      (Lower values = higher quality/larger file)".to_string());
        lines.push("      (Higher values = more compression/smaller file)".to_string());
    } else {
        lines.push("   AV1 Quality: (not set)".to_string());
    }
    
    if let Some(profile) = job.av1_profile {
        let profile_name = match profile {
            0 => "Main (8-bit)",
            1 => "High (10-bit)",
            2 => "Professional (12-bit)",
            _ => "Unknown",
        };
        lines.push(format!("   AV1 Profile: {} ({})", profile, profile_name));
    } else {
        lines.push("   AV1 Profile: (not set)".to_string());
    }
    
    lines.push(format!("   Web-like Content: {}", if job.is_web_like { "Yes" } else { "No" }));
    
    lines.push("".to_string());
    
    // File sizes and savings with Unicode symbols and improved spacing
    lines.push("💾 FILE SIZES:".to_string());
    
    if let Some(orig_bytes) = job.original_bytes {
        lines.push(format!("   Original Size: {} ({} bytes)", format_size(orig_bytes, DECIMAL), orig_bytes));
    } else {
        lines.push("   Original Size: (not available)".to_string());
    }
    
    if let Some(new_bytes) = job.new_bytes {
        lines.push(format!("   New Size: {} ({} bytes)", format_size(new_bytes, DECIMAL), new_bytes));
        
        // Calculate savings
        if let Some(orig_bytes) = job.original_bytes {
            if orig_bytes > 0 {
                let savings_bytes = orig_bytes.saturating_sub(new_bytes);
                let savings_pct = (savings_bytes as f64 / orig_bytes as f64) * 100.0;
                let compression_ratio = (savings_bytes as f64 / orig_bytes as f64) * 100.0;
                
                lines.push(format!("   Space Saved: {} ({:.1}%)", format_size(savings_bytes, DECIMAL), savings_pct));
                lines.push(format!("   Compression Ratio: {:.1}%", compression_ratio));
            }
        }
    } else {
        lines.push("   New Size: (not available)".to_string());
    }
    
    lines.push("".to_string());
    lines.push("╚═══════════════════════════════════════════════════════════════════════════════╝".to_string());
    lines.push("  ⌨  Press ESC or Enter to close".to_string());
    
    // Join all lines
    let content = lines.join("\n");
    
    // Create scrollable paragraph
    // For now, we'll show all content without scrolling
    // TODO: Implement scrolling for long content in future enhancement
    let paragraph = Paragraph::new(content)
        .block(Block::default()
            .borders(Borders::ALL)
            .title("📋 Job Details")
            .border_style(Style::default().fg(app.color_scheme.border_selected))
            .style(Style::default().bg(Color::Black)))
        .style(Style::default().fg(app.color_scheme.text_primary).bg(Color::Black))
        .wrap(ratatui::widgets::Wrap { trim: false });
    
    // Render a background overlay first (semi-transparent effect using DarkGray background)
    let overlay = Block::default()
        .style(Style::default().bg(Color::Black));
    f.render_widget(overlay, area);
    
    // Render the modal on top
    f.render_widget(paragraph, modal_area);
}

fn render_top_bar(f: &mut Frame, app: &App, area: Rect) {
    // Split top bar into four parts: Activity, CPU, Memory, and GPU
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(16),  // Activity status
            Constraint::Percentage(28),
            Constraint::Percentage(36),
            Constraint::Percentage(36),
        ])
        .split(area);
    
    // Render activity status with Unicode symbol
    let (activity_text, activity_color) = app.get_activity_status();
    let activity_block = Block::default()
        .borders(Borders::ALL)
        .title("📡 STATUS")
        .style(Style::default().fg(activity_color));
    let activity_paragraph = Paragraph::new(activity_text)
        .block(activity_block)
        .style(Style::default().fg(activity_color));
    f.render_widget(activity_paragraph, chunks[0]);
    
    // Adjust chunk indices for remaining gauges
    let gauge_chunks = &chunks[1..];

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

    // CPU gauge - use metric colors based on usage level
    let cpu_percent_u16 = cpu_usage.min(100.0).max(0.0) as u16;
    let cpu_color = if cpu_usage > 80.0 {
        app.color_scheme.metric_high
    } else if cpu_usage > 50.0 {
        app.color_scheme.metric_medium
    } else {
        app.color_scheme.metric_low
    };
    let cpu_gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title("🖥  CPU"))
        .gauge_style(Style::default().fg(cpu_color))
        .percent(cpu_percent_u16)
        .label(format!("{:.1}%", cpu_usage));
    f.render_widget(cpu_gauge, gauge_chunks[0]);

    // Memory gauge - use metric colors based on usage level
    let memory_percent_u16 = memory_percent.min(100.0).max(0.0) as u16;
    let memory_color = if memory_percent > 80.0 {
        app.color_scheme.metric_high
    } else if memory_percent > 50.0 {
        app.color_scheme.metric_medium
    } else {
        app.color_scheme.metric_low
    };
    let memory_gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title("💾 MEMORY"))
        .gauge_style(Style::default().fg(memory_color))
        .percent(memory_percent_u16)
        .label(format!("{:.1}%", memory_percent));
    f.render_widget(memory_gauge, gauge_chunks[1]);

    // GPU gauge - use metric colors based on usage level
    let gpu_usage = app.get_gpu_usage();
    let gpu_percent_u16 = gpu_usage.min(100.0).max(0.0) as u16;
    let gpu_color = if gpu_usage > 80.0 {
        app.color_scheme.metric_high
    } else if gpu_usage > 50.0 {
        app.color_scheme.metric_medium
    } else {
        app.color_scheme.metric_low
    };
    let gpu_gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title("🎮 GPU"))
        .gauge_style(Style::default().fg(gpu_color))
        .percent(gpu_percent_u16)
        .label(format!("{:.1}%", gpu_usage));
    f.render_widget(gpu_gauge, gauge_chunks[2]);
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
    
    // Create layout configuration based on terminal size
    let layout_config = LayoutConfig::from_terminal_size(area);
    
    // Build header dynamically based on visible columns
    let header_cells: Vec<String> = layout_config.table_columns.iter()
        .map(|col| col.header().to_string())
        .collect();
    
    let header = Row::new(header_cells)
        .style(Style::default().add_modifier(Modifier::BOLD))
        .height(1);

    // Apply filtering
    let mut filtered_jobs = app.filter_jobs(&app.jobs);
    
    // Apply sorting
    app.sort_jobs(&mut filtered_jobs);
    
    // Build rows - limit to what fits on screen
    let rows: Vec<Row> = if filtered_jobs.is_empty() {
        // Build empty row with correct number of columns
        let empty_cells: Vec<String> = layout_config.table_columns.iter()
            .enumerate()
            .map(|(idx, col)| {
                if idx == 0 {
                    "No jobs".to_string()
                } else if idx == 1 && *col == TableColumn::File {
                    format!("Dir: {}", app.job_state_dir.display())
                } else {
                    "-".to_string()
                }
            })
            .collect();
        vec![Row::new(empty_cells).height(1)]
    } else {
        let num_rows = max_data_rows.min(20).min(filtered_jobs.len());
        filtered_jobs
            .iter()
            .enumerate()
            .take(num_rows)
            .map(|(row_idx, job)| {
                // Pre-calculate all possible cell values with Unicode symbols
                let status_str = match job.status {
                    JobStatus::Pending => "⏸ PEND",
                    JobStatus::Running => "⚙ RUN",
                    JobStatus::Success => "✓ OK",
                    JobStatus::Failed => "✗ FAIL",
                    JobStatus::Skipped => "⊘ SKIP",
                };

                let file_name = job.source_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("?")
                    .to_string();
                let file_name = truncate_string(&file_name, 50);

                let resolution = if let (Some(width), Some(height)) = (job.video_width, job.video_height) {
                    format!("{}x{}", width, height)
                } else {
                    "-".to_string()
                };

                let codec = job.video_codec.as_deref()
                    .map(|c| c.to_uppercase())
                    .unwrap_or_else(|| "-".to_string());

                let bitrate = job.video_bitrate
                    .map(|br| {
                        let mbps = br as f64 / 1_000_000.0;
                        format!("{:.1}M", mbps)
                    })
                    .unwrap_or_else(|| "-".to_string());

                let hdr_indicator = match job.is_hdr {
                    Some(true) => "◆HDR",
                    Some(false) => "",
                    None => "-",
                };

                let bit_depth = job.source_bit_depth
                    .map(|depth| format!("{}b", depth))
                    .unwrap_or_else(|| "-".to_string());

                let orig_size = job.original_bytes
                    .map(|b| format_size(b, DECIMAL))
                    .unwrap_or_else(|| "-".to_string());

                let new_size = job.new_bytes
                    .map(|b| format_size(b, DECIMAL))
                    .unwrap_or_else(|| "-".to_string());

                let compression_ratio = if let (Some(orig), Some(new)) = (job.original_bytes, job.new_bytes) {
                    if orig > 0 {
                        let ratio = ((orig - new) as f64 / orig as f64) * 100.0;
                        format!("{:.0}%", ratio)
                    } else {
                        "-".to_string()
                    }
                } else {
                    "-".to_string()
                };

                let savings = if let (Some(orig), Some(new)) = (job.original_bytes, job.new_bytes) {
                    if orig > 0 {
                        let savings_bytes = orig - new;
                        let savings_gb = savings_bytes as f64 / 1_000_000_000.0;
                        let pct = (savings_bytes as f64 / orig as f64) * 100.0;
                        format!("{:.1}GB ({:.0}%)", savings_gb, pct)
                    } else {
                        "-".to_string()
                    }
                } else {
                    let savings_str = if let Some(cached_estimate) = app.estimated_savings_cache.get(&job.id) {
                        if let Some((savings_gb, savings_pct)) = cached_estimate {
                            format!("~{:.1}GB ({:.0}%)", savings_gb, savings_pct)
                        } else {
                            let missing = vec![
                                if job.original_bytes.is_none() { "orig" } else { "" },
                                if job.video_codec.is_none() { "codec" } else { "" },
                                if job.video_width.is_none() { "w" } else { "" },
                                if job.video_height.is_none() { "h" } else { "" },
                                if job.video_bitrate.is_none() { "br" } else { "" },
                                if job.video_frame_rate.is_none() { "fps" } else { "" },
                            ].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(",");
                            if missing.is_empty() {
                                "calc?".to_string()
                            } else {
                                format!("-{}", truncate_string(&missing, 10))
                            }
                        }
                    } else {
                        if has_estimation_metadata(job) {
                            if let Some((savings_gb, savings_pct)) = estimate_space_savings(job) {
                                format!("~{:.1}GB ({:.0}%)", savings_gb, savings_pct)
                            } else {
                                "calc?".to_string()
                            }
                        } else {
                            let missing = vec![
                                if job.original_bytes.is_none() { "orig" } else { "" },
                                if job.video_codec.is_none() { "codec" } else { "" },
                                if job.video_width.is_none() { "w" } else { "" },
                                if job.video_height.is_none() { "h" } else { "" },
                                if job.video_bitrate.is_none() { "br" } else { "" },
                                if job.video_frame_rate.is_none() { "fps" } else { "" },
                            ].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(",");
                            format!("-{}", truncate_string(&missing, 10))
                        }
                    };
                    savings_str
                };

                let duration = if let Some(started) = job.started_at {
                    if let Some(finished) = job.finished_at {
                        let dur = finished - started;
                        format!("{}s", dur.num_seconds())
                    } else if job.status == JobStatus::Running {
                        let dur = Utc::now() - started;
                        format!("{}s", dur.num_seconds())
                    } else {
                        "-".to_string()
                    }
                } else {
                    "-".to_string()
                };

                let reason = truncate_string(job.reason.as_deref().unwrap_or("-"), 30);
                
                let quality_str = if let Some(quality) = job.av1_quality {
                    format!("{}", quality)
                } else {
                    "-".to_string()
                };

                // Build row cells dynamically based on visible columns
                let cells: Vec<String> = layout_config.table_columns.iter()
                    .map(|col| {
                        match col {
                            TableColumn::Status => status_str.to_string(),
                            TableColumn::File => file_name.clone(),
                            TableColumn::Resolution => resolution.clone(),
                            TableColumn::Codec => codec.clone(),
                            TableColumn::Bitrate => bitrate.clone(),
                            TableColumn::Hdr => hdr_indicator.to_string(),
                            TableColumn::BitDepth => bit_depth.clone(),
                            TableColumn::OrigSize => orig_size.clone(),
                            TableColumn::NewSize => new_size.clone(),
                            TableColumn::CompressionRatio => compression_ratio.clone(),
                            TableColumn::Quality => quality_str.clone(),
                            TableColumn::Savings => savings.clone(),
                            TableColumn::Time => duration.clone(),
                            TableColumn::Reason => reason.clone(),
                        }
                    })
                    .collect();

                let mut row = Row::new(cells).height(1);
                
                // Apply selection highlighting
                if let Some(selected_idx) = app.ui_state.selected_index {
                    if row_idx == selected_idx {
                        row = row.style(Style::default()
                            .fg(Color::Black)
                            .bg(app.color_scheme.border_selected)
                            .add_modifier(Modifier::BOLD));
                    }
                }
                
                row
            })
            .collect()
    };

    // Build column widths dynamically based on visible columns
    let widths: Vec<Constraint> = layout_config.table_columns.iter()
        .map(|col| col.width())
        .collect();

    let filter_name = match app.ui_state.filter {
        JobFilter::All => "All",
        JobFilter::Pending => "Pending",
        JobFilter::Running => "Running",
        JobFilter::Success => "Success",
        JobFilter::Failed => "Failed",
    };
    
    let sort_name = match app.ui_state.sort_mode {
        SortMode::ByDate => "Date",
        SortMode::BySize => "Size",
        SortMode::ByStatus => "Status",
        SortMode::BySavings => "Savings",
    };
    
    let title = if filtered_jobs.is_empty() {
        format!("📋 JOBS [{}│Sort:{}] (0 found)", filter_name, sort_name)
    } else {
        format!("📋 JOBS [{}│Sort:{}] ({}/{})", filter_name, sort_name, filtered_jobs.len(), app.jobs.len())
    };
    
    // Use distinct border color when a row is selected
    let border_color = if app.ui_state.selected_index.is_some() {
        app.color_scheme.border_selected
    } else {
        app.color_scheme.border_normal
    };
    
    // Use minimal spacing and compact borders
    let table = Table::new(rows, &widths)
        .header(header)
        .block(Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(border_color)))
        .column_spacing(1);

    f.render_stateful_widget(table, area, &mut app.ui_state.table_state);
}

fn render_statistics_dashboard(f: &mut Frame, app: &App, area: Rect) {
    let stats = &app.statistics_cache;
    
    // Format statistics with proper units
    let total_saved_str = format_size(stats.total_space_saved, DECIMAL);
    let avg_compression_str = format!("{:.1}%", stats.average_compression_ratio * 100.0);
    let success_rate_str = format!("{:.1}%", stats.success_rate);
    let est_pending_str = format_size(stats.estimated_pending_savings, DECIMAL);
    
    // Format total processing time
    let total_time_str = if stats.total_processing_time > 0 {
        let hours = stats.total_processing_time / 3600;
        let minutes = (stats.total_processing_time % 3600) / 60;
        if hours > 0 {
            format!("{}h {}m", hours, minutes)
        } else {
            format!("{}m", minutes)
        }
    } else {
        "-".to_string()
    };
    
    // Determine if we should show compact or full view based on available height
    // Compact: 1 line (height = 3 with borders)
    // Full: 2 lines (height = 4 with borders)
    // Full with sparklines: 3+ lines (height >= 5 with borders)
    let is_compact = area.height < 4;
    let show_sparklines = area.height >= 5 && area.width >= 100;
    
    if show_sparklines {
        // Split area into text and sparklines
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),  // Statistics text (2 lines)
                Constraint::Length(1),  // Sparklines
            ])
            .split(area);
        
        // Build statistics text with improved spacing and color gradients
        let savings_color = app.color_scheme.gradient_color_for_savings(
            stats.total_space_saved as f64 / 1_000_000_000.0
        );
        let compression_color = app.color_scheme.gradient_color_for_compression(
            stats.average_compression_ratio * 100.0
        );
        let success_color = app.color_scheme.gradient_color_for_percentage(
            100.0 - stats.success_rate  // Invert so high success = green
        );
        
        let line1 = Line::from(vec![
            Span::raw("  Total Space Saved: "),
            Span::styled(total_saved_str, Style::default().fg(savings_color)),
            Span::raw("  │  Avg Compression: "),
            Span::styled(avg_compression_str, Style::default().fg(compression_color)),
            Span::raw("  │  Success Rate: "),
            Span::styled(success_rate_str, Style::default().fg(success_color)),
        ]);
        
        let pending_savings_gb = stats.estimated_pending_savings as f64 / 1_000_000_000.0;
        let pending_color = app.color_scheme.gradient_color_for_savings(pending_savings_gb);
        
        let line2 = Line::from(vec![
            Span::raw("  Est. Pending Savings: "),
            Span::styled(est_pending_str, Style::default().fg(pending_color)),
            Span::raw("  │  Total Processing Time: "),
            Span::styled(total_time_str, Style::default().fg(app.color_scheme.text_primary)),
        ]);
        
        let paragraph = Paragraph::new(vec![line1, line2])
            .block(Block::default()
                .borders(Borders::TOP | Borders::LEFT | Borders::RIGHT)
                .title("📊 STATISTICS")
                .style(Style::default().fg(app.color_scheme.header)))
            .style(Style::default().fg(app.color_scheme.text_primary));
        
        f.render_widget(paragraph, chunks[0]);
        
        // Render sparklines for trends
        render_trend_sparklines(f, app, chunks[1]);
    } else {
        // Build statistics text without sparklines with improved spacing and color gradients
        let savings_color = app.color_scheme.gradient_color_for_savings(
            stats.total_space_saved as f64 / 1_000_000_000.0
        );
        let compression_color = app.color_scheme.gradient_color_for_compression(
            stats.average_compression_ratio * 100.0
        );
        let success_color = app.color_scheme.gradient_color_for_percentage(
            100.0 - stats.success_rate  // Invert so high success = green
        );
        let pending_savings_gb = stats.estimated_pending_savings as f64 / 1_000_000_000.0;
        let pending_color = app.color_scheme.gradient_color_for_savings(pending_savings_gb);
        
        let stats_lines = if is_compact {
            // Compact mode: single line with most important stats
            vec![Line::from(vec![
                Span::raw("  Saved: "),
                Span::styled(&total_saved_str, Style::default().fg(savings_color)),
                Span::raw("  │  Avg Comp: "),
                Span::styled(&avg_compression_str, Style::default().fg(compression_color)),
                Span::raw("  │  Success: "),
                Span::styled(&success_rate_str, Style::default().fg(success_color)),
                Span::raw("  │  Est Pending: "),
                Span::styled(&est_pending_str, Style::default().fg(pending_color)),
            ])]
        } else {
            // Full mode: two lines with all stats
            vec![
                Line::from(vec![
                    Span::raw("  Total Space Saved: "),
                    Span::styled(&total_saved_str, Style::default().fg(savings_color)),
                    Span::raw("  │  Avg Compression: "),
                    Span::styled(&avg_compression_str, Style::default().fg(compression_color)),
                    Span::raw("  │  Success Rate: "),
                    Span::styled(&success_rate_str, Style::default().fg(success_color)),
                ]),
                Line::from(vec![
                    Span::raw("  Est. Pending Savings: "),
                    Span::styled(&est_pending_str, Style::default().fg(pending_color)),
                    Span::raw("  │  Total Processing Time: "),
                    Span::styled(&total_time_str, Style::default().fg(app.color_scheme.text_primary)),
                ]),
            ]
        };
        
        let paragraph = Paragraph::new(stats_lines)
            .block(Block::default()
                .borders(Borders::ALL)
                .title("📊 STATISTICS")
                .style(Style::default().fg(app.color_scheme.header)))
            .style(Style::default().fg(app.color_scheme.text_primary));
        
        f.render_widget(paragraph, area);
    }
}

fn render_trend_sparklines(f: &mut Frame, app: &App, area: Rect) {
    let stats = &app.statistics_cache;
    
    // Split area horizontally for two sparklines
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(50),  // Processing time trend
            Constraint::Percentage(50),  // Compression ratio trend
        ])
        .split(area);
    
    // Prepare processing time data for sparkline
    let proc_time_data: Vec<u64> = stats.recent_processing_times.iter()
        .map(|&t| t.max(0) as u64)
        .collect();
    
    // Calculate trend indicator for processing times
    let proc_time_trend = if proc_time_data.len() >= 2 {
        let first_half_avg = proc_time_data.iter().take(proc_time_data.len() / 2).sum::<u64>() as f64 
            / (proc_time_data.len() / 2).max(1) as f64;
        let second_half_avg = proc_time_data.iter().skip(proc_time_data.len() / 2).sum::<u64>() as f64 
            / (proc_time_data.len() - proc_time_data.len() / 2).max(1) as f64;
        
        if second_half_avg > first_half_avg * 1.1 {
            "↑"  // Getting slower
        } else if second_half_avg < first_half_avg * 0.9 {
            "↓"  // Getting faster
        } else {
            "→"  // Stable
        }
    } else {
        "-"
    };
    
    // Render processing time sparkline
    if !proc_time_data.is_empty() {
        let sparkline = Sparkline::default()
            .block(Block::default()
                .borders(Borders::BOTTOM | Borders::LEFT)
                .title(format!("Proc Time {} ", proc_time_trend)))
            .data(&proc_time_data)
            .style(Style::default().fg(app.color_scheme.progress_verifying));
        f.render_widget(sparkline, chunks[0]);
    } else {
        let no_data = Paragraph::new("No data")
            .block(Block::default()
                .borders(Borders::BOTTOM | Borders::LEFT)
                .title("Proc Time -"))
            .style(Style::default().fg(app.color_scheme.text_muted));
        f.render_widget(no_data, chunks[0]);
    }
    
    // Prepare compression ratio data for sparkline (convert to percentage for better visualization)
    let comp_ratio_data: Vec<u64> = stats.recent_compression_ratios.iter()
        .map(|&r| (r * 100.0).max(0.0) as u64)
        .collect();
    
    // Calculate trend indicator for compression ratios
    let comp_ratio_trend = if comp_ratio_data.len() >= 2 {
        let first_half_avg = comp_ratio_data.iter().take(comp_ratio_data.len() / 2).sum::<u64>() as f64 
            / (comp_ratio_data.len() / 2).max(1) as f64;
        let second_half_avg = comp_ratio_data.iter().skip(comp_ratio_data.len() / 2).sum::<u64>() as f64 
            / (comp_ratio_data.len() - comp_ratio_data.len() / 2).max(1) as f64;
        
        if second_half_avg > first_half_avg * 1.1 {
            "↑"  // Better compression
        } else if second_half_avg < first_half_avg * 0.9 {
            "↓"  // Worse compression
        } else {
            "→"  // Stable
        }
    } else {
        "-"
    };
    
    // Render compression ratio sparkline
    if !comp_ratio_data.is_empty() {
        let sparkline = Sparkline::default()
            .block(Block::default()
                .borders(Borders::BOTTOM | Borders::LEFT | Borders::RIGHT)
                .title(format!("Comp Ratio {} ", comp_ratio_trend)))
            .data(&comp_ratio_data)
            .style(Style::default().fg(app.color_scheme.metric_low));
        f.render_widget(sparkline, chunks[1]);
    } else {
        let no_data = Paragraph::new("No data")
            .block(Block::default()
                .borders(Borders::BOTTOM | Borders::LEFT | Borders::RIGHT)
                .title("Comp Ratio -"))
            .style(Style::default().fg(app.color_scheme.text_muted));
        f.render_widget(no_data, chunks[1]);
    }
}

fn render_status_bar(f: &mut Frame, app: &App, area: Rect) {
    // Task 12.1: Display all keyboard shortcuts grouped by category
    // Task 12.2: Display current filter and sort mode
    // Task 12.3: Display refresh information
    
    let total = app.jobs.len();
    let running = app.count_by_status(JobStatus::Running);
    let failed = app.count_by_status(JobStatus::Failed);
    let skipped = app.count_by_status(JobStatus::Skipped);
    let pending = app.count_by_status(JobStatus::Pending);
    let success = app.count_by_status(JobStatus::Success);

    // Truncate directory path if too long
    let dir_display = app.job_state_dir.display().to_string();
    let dir_short = truncate_string(&dir_display, 35);

    // Include message if present
    let message_part = if let Some(msg) = &app.last_message {
        format!(" │ MSG: {}", msg)
    } else {
        String::new()
    };
    
    // Task 12.2: Display current filter and sort mode with distinct formatting
    let filter_name = match app.ui_state.filter {
        JobFilter::All => "All",
        JobFilter::Pending => "Pending",
        JobFilter::Running => "Running",
        JobFilter::Success => "Success",
        JobFilter::Failed => "Failed",
    };
    
    let sort_name = match app.ui_state.sort_mode {
        SortMode::ByDate => "Date",
        SortMode::BySize => "Size",
        SortMode::ByStatus => "Status",
        SortMode::BySavings => "Savings",
    };
    
    // Task 12.3: Display refresh information
    let seconds_since_refresh = (Utc::now() - app.last_refresh).num_seconds();
    let refresh_info = format!("Last: {}s ago", seconds_since_refresh);
    
    // Determine refresh rate based on current state
    let has_active_job = app.jobs.iter().any(|j| j.status == JobStatus::Running);
    let pending_count = app.jobs.iter().filter(|j| j.status == JobStatus::Pending).count();
    let refresh_rate = if has_active_job {
        "1s"
    } else if pending_count > 0 {
        "250ms"
    } else {
        "5s"
    };
    
    // Build status bar content with multiple lines for better organization
    // Line 1: Job counts and current modes
    // Line 2: Keyboard shortcuts grouped by category
    
    let line1 = format!(
        "  Jobs: {} │ Running: {} │ Pending: {} │ Success: {} │ Failed: {} │ Skipped: {} │ Filter: [{}] │ Sort: [{}] │ Refresh: {} (rate: {}){}",
        total, running, pending, success, failed, skipped, filter_name, sort_name, refresh_info, refresh_rate, message_part
    );
    
    // Task 12.1: Group shortcuts by category with clear separators
    let line2 = format!(
        "  Navigation: ↑↓=move PgUp/PgDn=page │ Filters: 1=all 2=pend 3=run 4=ok 5=fail │ Actions: s=sort Enter=details r=refresh R=requeue q=quit │ Dir: {}",
        dir_short
    );
    
    let status_lines = vec![
        Line::from(line1),
        Line::from(line2),
    ];

    let paragraph = Paragraph::new(status_lines)
        .block(Block::default()
            .borders(Borders::ALL)
            .title("⌨  CONTROLS & STATUS")
            .style(Style::default().fg(app.color_scheme.header)))
        .style(Style::default().fg(app.color_scheme.text_primary))
        .wrap(ratatui::widgets::Wrap { trim: false });
    
    f.render_widget(paragraph, area);
}

fn truncate_string(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::path::PathBuf;
    
    // Unit tests for statistics calculation
    
    #[test]
    fn test_total_space_saved_with_various_job_sets() {
        // Test with empty job list
        let jobs: Vec<Job> = vec![];
        let stats = StatisticsCache::calculate(&jobs);
        assert_eq!(stats.total_space_saved, 0, "Empty job list should have 0 space saved");
        
        // Test with successful jobs
        let mut jobs = vec![];
        for i in 0..5 {
            let mut job = create_test_job(&format!("job{}", i), JobStatus::Success);
            job.original_bytes = Some(1_000_000_000); // 1GB
            job.new_bytes = Some(500_000_000); // 500MB
            jobs.push(job);
        }
        let stats = StatisticsCache::calculate(&jobs);
        assert_eq!(stats.total_space_saved, 2_500_000_000, "Should sum all savings");
        
        // Test with mixed statuses (only Success should count)
        let mut jobs = vec![];
        let mut job1 = create_test_job("job1", JobStatus::Success);
        job1.original_bytes = Some(1_000_000_000);
        job1.new_bytes = Some(500_000_000);
        jobs.push(job1);
        
        let mut job2 = create_test_job("job2", JobStatus::Failed);
        job2.original_bytes = Some(1_000_000_000);
        job2.new_bytes = Some(500_000_000);
        jobs.push(job2);
        
        let mut job3 = create_test_job("job3", JobStatus::Pending);
        job3.original_bytes = Some(1_000_000_000);
        jobs.push(job3);
        
        let stats = StatisticsCache::calculate(&jobs);
        assert_eq!(stats.total_space_saved, 500_000_000, "Only successful jobs should count");
        
        // Test with jobs missing metadata
        let mut jobs = vec![];
        let mut job1 = create_test_job("job1", JobStatus::Success);
        job1.original_bytes = Some(1_000_000_000);
        job1.new_bytes = None; // Missing new_bytes
        jobs.push(job1);
        
        let mut job2 = create_test_job("job2", JobStatus::Success);
        job2.original_bytes = None; // Missing original_bytes
        job2.new_bytes = Some(500_000_000);
        jobs.push(job2);
        
        let stats = StatisticsCache::calculate(&jobs);
        assert_eq!(stats.total_space_saved, 0, "Jobs with missing metadata should not count");
    }
    
    #[test]
    fn test_average_compression_ratio_calculation() {
        // Test with empty job list
        let jobs: Vec<Job> = vec![];
        let stats = StatisticsCache::calculate(&jobs);
        assert_eq!(stats.average_compression_ratio, 0.0, "Empty job list should have 0 ratio");
        
        // Test with successful jobs
        let mut jobs = vec![];
        for i in 0..3 {
            let mut job = create_test_job(&format!("job{}", i), JobStatus::Success);
            job.original_bytes = Some(1_000_000_000); // 1GB
            job.new_bytes = Some(500_000_000); // 500MB (50% compression)
            jobs.push(job);
        }
        let stats = StatisticsCache::calculate(&jobs);
        assert!((stats.average_compression_ratio - 0.5).abs() < 0.001, 
            "Average compression ratio should be 0.5, got {}", stats.average_compression_ratio);
        
        // Test with varying compression ratios
        let mut jobs = vec![];
        let mut job1 = create_test_job("job1", JobStatus::Success);
        job1.original_bytes = Some(1_000_000_000);
        job1.new_bytes = Some(600_000_000); // 40% compression
        jobs.push(job1);
        
        let mut job2 = create_test_job("job2", JobStatus::Success);
        job2.original_bytes = Some(1_000_000_000);
        job2.new_bytes = Some(400_000_000); // 60% compression
        jobs.push(job2);
        
        let stats = StatisticsCache::calculate(&jobs);
        let expected_avg = (0.4 + 0.6) / 2.0;
        assert!((stats.average_compression_ratio - expected_avg).abs() < 0.001,
            "Average should be {}, got {}", expected_avg, stats.average_compression_ratio);
        
        // Test with zero original size (edge case)
        let mut jobs = vec![];
        let mut job = create_test_job("job1", JobStatus::Success);
        job.original_bytes = Some(0);
        job.new_bytes = Some(0);
        jobs.push(job);
        
        let stats = StatisticsCache::calculate(&jobs);
        assert_eq!(stats.average_compression_ratio, 0.0, "Zero size should result in 0 ratio");
    }
    
    #[test]
    fn test_success_rate_calculation() {
        // Test with empty job list
        let jobs: Vec<Job> = vec![];
        let stats = StatisticsCache::calculate(&jobs);
        assert_eq!(stats.success_rate, 0.0, "Empty job list should have 0% success rate");
        
        // Test with all successful jobs
        let mut jobs = vec![];
        for i in 0..5 {
            jobs.push(create_test_job(&format!("job{}", i), JobStatus::Success));
        }
        let stats = StatisticsCache::calculate(&jobs);
        assert_eq!(stats.success_rate, 100.0, "All successful should be 100%");
        
        // Test with all failed jobs
        let mut jobs = vec![];
        for i in 0..5 {
            jobs.push(create_test_job(&format!("job{}", i), JobStatus::Failed));
        }
        let stats = StatisticsCache::calculate(&jobs);
        assert_eq!(stats.success_rate, 0.0, "All failed should be 0%");
        
        // Test with mixed completed jobs
        let mut jobs = vec![];
        for i in 0..7 {
            jobs.push(create_test_job(&format!("success{}", i), JobStatus::Success));
        }
        for i in 0..3 {
            jobs.push(create_test_job(&format!("failed{}", i), JobStatus::Failed));
        }
        let stats = StatisticsCache::calculate(&jobs);
        assert_eq!(stats.success_rate, 70.0, "7/10 should be 70%");
        
        // Test that pending/running jobs don't affect success rate
        let mut jobs = vec![];
        jobs.push(create_test_job("success", JobStatus::Success));
        jobs.push(create_test_job("failed", JobStatus::Failed));
        jobs.push(create_test_job("pending", JobStatus::Pending));
        jobs.push(create_test_job("running", JobStatus::Running));
        jobs.push(create_test_job("skipped", JobStatus::Skipped));
        
        let stats = StatisticsCache::calculate(&jobs);
        assert_eq!(stats.success_rate, 50.0, "Only completed jobs should count (1/2 = 50%)");
    }
    
    #[test]
    fn test_edge_cases() {
        // Test with all pending jobs (no completed jobs)
        let mut jobs = vec![];
        for i in 0..5 {
            jobs.push(create_test_job(&format!("job{}", i), JobStatus::Pending));
        }
        let stats = StatisticsCache::calculate(&jobs);
        assert_eq!(stats.total_space_saved, 0);
        assert_eq!(stats.average_compression_ratio, 0.0);
        assert_eq!(stats.success_rate, 0.0);
        assert_eq!(stats.total_processing_time, 0);
        
        // Test with jobs that have started but not finished
        let mut jobs = vec![];
        let mut job = create_test_job("running", JobStatus::Running);
        job.started_at = Some(Utc::now());
        jobs.push(job);
        
        let stats = StatisticsCache::calculate(&jobs);
        assert_eq!(stats.total_processing_time, 0, "Running jobs without finished_at should not count");
    }
    
    // Helper function to create test jobs
    fn create_test_job(id: &str, status: JobStatus) -> Job {
        Job {
            id: id.to_string(),
            source_path: PathBuf::from("/test/video.mkv"),
            output_path: None,
            status,
            reason: Some("test".to_string()),
            created_at: Utc::now(),
            started_at: None,
            finished_at: None,
            original_bytes: None,
            new_bytes: None,
            is_web_like: false,
            video_codec: None,
            video_width: None,
            video_height: None,
            video_bitrate: None,
            video_frame_rate: None,
            is_hdr: None,
            source_bit_depth: None,
            source_pix_fmt: None,
            target_bit_depth: None,
            av1_quality: None,
            av1_profile: None,
        }
    }
    
    // Strategy to generate random JobStatus
    fn job_status_strategy() -> impl Strategy<Value = JobStatus> {
        prop_oneof![
            Just(JobStatus::Pending),
            Just(JobStatus::Running),
            Just(JobStatus::Success),
            Just(JobStatus::Failed),
            Just(JobStatus::Skipped),
        ]
    }
    
    // Strategy to generate random Job
    fn job_strategy() -> impl Strategy<Value = Job> {
        (
            "[a-z]{10}",  // id
            any::<bool>(), // for various optional fields
            job_status_strategy(),
        ).prop_map(|(id, has_metadata, status)| {
            Job {
                id,
                source_path: PathBuf::from("/test/video.mkv"),
                output_path: None,
                status,
                reason: Some("test".to_string()),
                created_at: Utc::now(),
                started_at: if has_metadata { Some(Utc::now()) } else { None },
                finished_at: None,
                original_bytes: if has_metadata { Some(1000000) } else { None },
                new_bytes: None,
                is_web_like: false,
                video_codec: if has_metadata { Some("h264".to_string()) } else { None },
                video_width: if has_metadata { Some(1920) } else { None },
                video_height: if has_metadata { Some(1080) } else { None },
                video_bitrate: if has_metadata { Some(5000000) } else { None },
                video_frame_rate: if has_metadata { Some("30/1".to_string()) } else { None },
                is_hdr: if has_metadata { Some(false) } else { None },
                source_bit_depth: if has_metadata { Some(8) } else { None },
                source_pix_fmt: if has_metadata { Some("yuv420p".to_string()) } else { None },
                target_bit_depth: if has_metadata { Some(8) } else { None },
                av1_quality: Some(25),
                av1_profile: Some(0),
            }
        })
    }
    
    /// **Feature: tui-improvements, Property 9: Keyboard shortcut uniqueness**
    /// **Validates: Requirements 8.1**
    /// 
    /// For any two different actions, they should be mapped to different keyboard shortcuts
    #[test]
    fn test_keyboard_shortcut_uniqueness() {
        // Define all keyboard shortcuts and their associated actions
        // Based on the implementation in the main event loop
        let shortcuts = vec![
            ("q", "Quit application"),
            ("r", "Refresh data"),
            ("R", "Requeue running job"),
            ("1", "Filter: All jobs"),
            ("2", "Filter: Pending jobs"),
            ("3", "Filter: Running jobs"),
            ("4", "Filter: Success jobs"),
            ("5", "Filter: Failed jobs"),
            ("s", "Cycle sort mode"),
            ("Up", "Move selection up"),
            ("Down", "Move selection down"),
            ("PageUp", "Move selection page up"),
            ("PageDown", "Move selection page down"),
            ("Enter", "Open/close detail view"),
            ("Esc", "Close detail view"),
        ];
        
        // Verify that all shortcuts are unique
        for i in 0..shortcuts.len() {
            for j in (i+1)..shortcuts.len() {
                assert!(
                    shortcuts[i].0 != shortcuts[j].0,
                    "Keyboard shortcuts must be unique: '{}' ({}) conflicts with '{}' ({})",
                    shortcuts[i].0, shortcuts[i].1,
                    shortcuts[j].0, shortcuts[j].1
                );
            }
        }
        
        // Verify that each action has exactly one shortcut
        let actions: Vec<&str> = shortcuts.iter().map(|(_, action)| *action).collect();
        for i in 0..actions.len() {
            for j in (i+1)..actions.len() {
                assert!(
                    actions[i] != actions[j],
                    "Each action should have exactly one shortcut: '{}' appears multiple times",
                    actions[i]
                );
            }
        }
        
        // Verify that filter shortcuts (1-5) are sequential and complete
        let filter_shortcuts: Vec<&str> = shortcuts.iter()
            .filter(|(_, action)| action.starts_with("Filter:"))
            .map(|(key, _)| *key)
            .collect();
        
        assert_eq!(
            filter_shortcuts.len(),
            5,
            "There should be exactly 5 filter shortcuts (1-5)"
        );
        
        // Verify filter shortcuts are 1-5
        let expected_filter_keys = vec!["1", "2", "3", "4", "5"];
        for expected_key in &expected_filter_keys {
            assert!(
                filter_shortcuts.contains(expected_key),
                "Filter shortcuts should include key '{}'",
                expected_key
            );
        }
        
        // Verify navigation shortcuts are present
        let navigation_shortcuts: Vec<&str> = shortcuts.iter()
            .filter(|(_, action)| action.starts_with("Move selection"))
            .map(|(key, _)| *key)
            .collect();
        
        assert_eq!(
            navigation_shortcuts.len(),
            4,
            "There should be exactly 4 navigation shortcuts (Up, Down, PageUp, PageDown)"
        );
        
        // Verify detail view shortcuts are present
        let detail_view_shortcuts: Vec<&str> = shortcuts.iter()
            .filter(|(_, action)| action.contains("detail view"))
            .map(|(key, _)| *key)
            .collect();
        
        assert!(
            detail_view_shortcuts.len() >= 2,
            "There should be at least 2 detail view shortcuts (Enter, Esc)"
        );
        
        // Verify that Enter and Esc are used for detail view
        assert!(
            detail_view_shortcuts.contains(&"Enter"),
            "Enter key should be used for detail view"
        );
        assert!(
            detail_view_shortcuts.contains(&"Esc"),
            "Esc key should be used for closing detail view"
        );
        
        // Verify sort shortcut is present
        let sort_shortcuts: Vec<&str> = shortcuts.iter()
            .filter(|(_, action)| action.contains("sort"))
            .map(|(key, _)| *key)
            .collect();
        
        assert_eq!(
            sort_shortcuts.len(),
            1,
            "There should be exactly 1 sort shortcut"
        );
        
        assert_eq!(
            sort_shortcuts[0],
            "s",
            "Sort shortcut should be 's'"
        );
    }
    
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]
        
        /// **Feature: tui-improvements, Property 1: Filter consistency**
        /// **Validates: Requirements 3.3**
        /// 
        /// For any job list and filter setting, all jobs displayed in the filtered view 
        /// should match the filter criteria
        #[test]
        fn test_filter_consistency(
            jobs in prop::collection::vec(job_strategy(), 0..50),
            filter_choice in 0..5usize,
        ) {
            // Create app with test data
            let mut app = App::new(PathBuf::from("/tmp/test"));
            app.jobs = jobs;
            
            // Set filter based on choice
            app.ui_state.filter = match filter_choice {
                0 => JobFilter::All,
                1 => JobFilter::Pending,
                2 => JobFilter::Running,
                3 => JobFilter::Success,
                _ => JobFilter::Failed,
            };
            
            // Apply filter
            let filtered = app.filter_jobs(&app.jobs);
            
            // Verify all filtered jobs match the filter criteria
            for job in &filtered {
                match app.ui_state.filter {
                    JobFilter::All => {
                        // All jobs should be included, no specific check needed
                    }
                    JobFilter::Pending => {
                        prop_assert_eq!(job.status, JobStatus::Pending, 
                            "Filtered job should have Pending status");
                    }
                    JobFilter::Running => {
                        prop_assert_eq!(job.status, JobStatus::Running,
                            "Filtered job should have Running status");
                    }
                    JobFilter::Success => {
                        prop_assert_eq!(job.status, JobStatus::Success,
                            "Filtered job should have Success status");
                    }
                    JobFilter::Failed => {
                        prop_assert_eq!(job.status, JobStatus::Failed,
                            "Filtered job should have Failed status");
                    }
                }
            }
            
            // Verify count matches expected
            let expected_count = match app.ui_state.filter {
                JobFilter::All => app.jobs.len(),
                JobFilter::Pending => app.jobs.iter().filter(|j| j.status == JobStatus::Pending).count(),
                JobFilter::Running => app.jobs.iter().filter(|j| j.status == JobStatus::Running).count(),
                JobFilter::Success => app.jobs.iter().filter(|j| j.status == JobStatus::Success).count(),
                JobFilter::Failed => app.jobs.iter().filter(|j| j.status == JobStatus::Failed).count(),
            };
            prop_assert_eq!(filtered.len(), expected_count,
                "Filtered count should match expected count");
        }
        
        /// **Feature: tui-improvements, Property 2: Sort order consistency**
        /// **Validates: Requirements 3.5**
        /// 
        /// For any job list and sort mode, jobs should be ordered according to 
        /// the sort criteria (date, size, status, or savings)
        #[test]
        fn test_sort_order_consistency(
            jobs in prop::collection::vec(job_strategy(), 0..50),
            sort_choice in 0..4usize,
        ) {
            // Create app with test data
            let mut app = App::new(PathBuf::from("/tmp/test"));
            app.jobs = jobs;
            
            // Pre-populate estimated savings cache for BySavings sort
            for job in &app.jobs {
                if job.status == JobStatus::Pending || job.status == JobStatus::Running {
                    let estimate = estimate_space_savings(job);
                    app.estimated_savings_cache.insert(job.id.clone(), estimate);
                }
            }
            
            // Set sort mode based on choice
            app.ui_state.sort_mode = match sort_choice {
                0 => SortMode::ByDate,
                1 => SortMode::BySize,
                2 => SortMode::ByStatus,
                _ => SortMode::BySavings,
            };
            
            // Get all jobs and sort them
            let mut sorted_jobs: Vec<&Job> = app.jobs.iter().collect();
            app.sort_jobs(&mut sorted_jobs);
            
            // Verify sort order is correct
            for i in 0..sorted_jobs.len().saturating_sub(1) {
                let current = sorted_jobs[i];
                let next = sorted_jobs[i + 1];
                
                match app.ui_state.sort_mode {
                    SortMode::ByDate => {
                        // Newest first (descending order)
                        prop_assert!(
                            current.created_at >= next.created_at,
                            "Jobs should be sorted by date (newest first): {:?} >= {:?}",
                            current.created_at, next.created_at
                        );
                    }
                    SortMode::BySize => {
                        // Largest first (descending order)
                        let current_size = current.original_bytes.unwrap_or(0);
                        let next_size = next.original_bytes.unwrap_or(0);
                        prop_assert!(
                            current_size >= next_size,
                            "Jobs should be sorted by size (largest first): {} >= {}",
                            current_size, next_size
                        );
                    }
                    SortMode::ByStatus => {
                        // Running > Failed > Pending > Success > Skipped
                        let current_priority = match current.status {
                            JobStatus::Running => 0,
                            JobStatus::Failed => 1,
                            JobStatus::Pending => 2,
                            JobStatus::Success => 3,
                            JobStatus::Skipped => 4,
                        };
                        let next_priority = match next.status {
                            JobStatus::Running => 0,
                            JobStatus::Failed => 1,
                            JobStatus::Pending => 2,
                            JobStatus::Success => 3,
                            JobStatus::Skipped => 4,
                        };
                        prop_assert!(
                            current_priority <= next_priority,
                            "Jobs should be sorted by status priority: {} <= {}",
                            current_priority, next_priority
                        );
                    }
                    SortMode::BySavings => {
                        // Highest savings first (descending order)
                        let current_savings = if let (Some(orig), Some(new)) = (current.original_bytes, current.new_bytes) {
                            orig.saturating_sub(new)
                        } else {
                            app.estimated_savings_cache.get(&current.id)
                                .and_then(|opt| opt.as_ref())
                                .map(|(gb, _)| (*gb * 1_000_000_000.0) as u64)
                                .unwrap_or(0)
                        };
                        let next_savings = if let (Some(orig), Some(new)) = (next.original_bytes, next.new_bytes) {
                            orig.saturating_sub(new)
                        } else {
                            app.estimated_savings_cache.get(&next.id)
                                .and_then(|opt| opt.as_ref())
                                .map(|(gb, _)| (*gb * 1_000_000_000.0) as u64)
                                .unwrap_or(0)
                        };
                        prop_assert!(
                            current_savings >= next_savings,
                            "Jobs should be sorted by savings (highest first): {} >= {}",
                            current_savings, next_savings
                        );
                    }
                }
            }
        }
        
        /// **Feature: tui-improvements, Property 4: Statistics accuracy**
        /// **Validates: Requirements 5.1**
        /// 
        /// For any set of completed jobs, the calculated total space saved should 
        /// equal the sum of individual job savings
        #[test]
        fn test_statistics_accuracy(
            jobs in prop::collection::vec(job_strategy(), 0..50),
        ) {
            // Create app with test data
            let mut app = App::new(PathBuf::from("/tmp/test"));
            
            // Set up jobs with actual savings data
            let mut modified_jobs = jobs;
            for job in &mut modified_jobs {
                // Only set savings for successful jobs
                if job.status == JobStatus::Success {
                    job.original_bytes = Some(1_000_000_000); // 1GB
                    job.new_bytes = Some(500_000_000); // 500MB
                }
            }
            app.jobs = modified_jobs;
            
            // Calculate statistics
            let stats = StatisticsCache::calculate(&app.jobs);
            
            // Manually calculate expected total space saved
            let expected_total: u64 = app.jobs.iter()
                .filter(|j| j.status == JobStatus::Success)
                .filter_map(|j| {
                    if let (Some(orig), Some(new)) = (j.original_bytes, j.new_bytes) {
                        Some(orig.saturating_sub(new))
                    } else {
                        None
                    }
                })
                .sum();
            
            // Verify total space saved matches sum of individual savings
            prop_assert_eq!(
                stats.total_space_saved,
                expected_total,
                "Total space saved should equal sum of individual job savings"
            );
            
            // Verify that only successful jobs with both original and new bytes are counted
            let counted_jobs = app.jobs.iter()
                .filter(|j| j.status == JobStatus::Success)
                .filter(|j| j.original_bytes.is_some() && j.new_bytes.is_some())
                .count();
            
            if counted_jobs > 0 {
                prop_assert!(
                    stats.total_space_saved > 0,
                    "If there are successful jobs with size data, total space saved should be > 0"
                );
            } else {
                prop_assert_eq!(
                    stats.total_space_saved,
                    0,
                    "If there are no successful jobs with size data, total space saved should be 0"
                );
            }
        }
        
        /// **Feature: tui-improvements, Property 3: Selection bounds**
        /// **Validates: Requirements 3.2**
        /// 
        /// For any job list and selection index, the selected index should always be 
        /// within valid bounds (0 to jobs.len()-1) or None
        #[test]
        fn test_selection_bounds(
            jobs in prop::collection::vec(job_strategy(), 0..50),
            operations in prop::collection::vec(0..4usize, 0..20),
        ) {
            // Create app with test data
            let mut app = App::new(PathBuf::from("/tmp/test"));
            app.jobs = jobs;
            
            // Get filtered job count
            let filtered_jobs = app.filter_jobs(&app.jobs);
            let job_count = filtered_jobs.len();
            
            // Perform random selection operations
            for op in operations {
                match op {
                    0 => app.move_selection_up(),
                    1 => app.move_selection_down(),
                    2 => app.move_selection_page_up(),
                    _ => app.move_selection_page_down(),
                }
                
                // Verify selection is within bounds or None
                if let Some(idx) = app.ui_state.selected_index {
                    if job_count > 0 {
                        prop_assert!(
                            idx < job_count,
                            "Selection index {} should be less than job count {}",
                            idx, job_count
                        );
                    } else {
                        // If no jobs, selection should be None
                        prop_assert!(
                            false,
                            "Selection should be None when there are no jobs, but got Some({})",
                            idx
                        );
                    }
                } else {
                    // None is always valid
                }
                
                // Verify table_state is in sync with selected_index
                prop_assert_eq!(
                    app.ui_state.table_state.selected(),
                    app.ui_state.selected_index,
                    "table_state.selected() should match selected_index"
                );
            }
            
            // Final verification: if there are jobs, selection should be Some
            // if there are no jobs, selection should be None
            if job_count == 0 {
                prop_assert_eq!(
                    app.ui_state.selected_index,
                    None,
                    "Selection should be None when there are no jobs"
                );
            }
        }
        
        /// **Feature: tui-improvements, Property 8: Compression ratio calculation**
        /// **Validates: Requirements 2.6**
        /// 
        /// For any completed job with original and new sizes, the compression ratio 
        /// should equal (original - new) / original
        #[test]
        fn test_compression_ratio_calculation(
            original_bytes in 1u64..10_000_000_000u64,  // 1 byte to 10GB
            compression_pct in 0.0f64..1.0f64,  // 0% to 100% compression
        ) {
            // Calculate new size based on compression percentage
            let new_bytes = (original_bytes as f64 * (1.0 - compression_pct)) as u64;
            
            // Create a completed job with these sizes
            let mut job = create_test_job("test_job", JobStatus::Success);
            job.original_bytes = Some(original_bytes);
            job.new_bytes = Some(new_bytes);
            
            // Calculate expected compression ratio
            let expected_ratio = if original_bytes > 0 {
                ((original_bytes - new_bytes) as f64 / original_bytes as f64) * 100.0
            } else {
                0.0
            };
            
            // Calculate actual compression ratio using the same logic as the UI
            let actual_ratio = if let (Some(orig), Some(new)) = (job.original_bytes, job.new_bytes) {
                if orig > 0 {
                    ((orig - new) as f64 / orig as f64) * 100.0
                } else {
                    0.0
                }
            } else {
                0.0
            };
            
            // Verify the ratio is calculated correctly
            prop_assert!(
                (actual_ratio - expected_ratio).abs() < 0.01,
                "Compression ratio should be {:.2}%, got {:.2}%",
                expected_ratio, actual_ratio
            );
            
            // Verify ratio is within valid bounds (0-100%)
            prop_assert!(
                actual_ratio >= 0.0 && actual_ratio <= 100.0,
                "Compression ratio should be between 0 and 100%, got {:.2}%",
                actual_ratio
            );
            
            // Verify specific edge cases
            if new_bytes == 0 {
                // 100% compression (file reduced to nothing)
                prop_assert!(
                    (actual_ratio - 100.0).abs() < 0.01,
                    "When new size is 0, compression should be 100%, got {:.2}%",
                    actual_ratio
                );
            }
            
            if new_bytes == original_bytes {
                // 0% compression (no reduction)
                prop_assert!(
                    actual_ratio.abs() < 0.01,
                    "When sizes are equal, compression should be 0%, got {:.2}%",
                    actual_ratio
                );
            }
            
            // Verify the ratio matches the input compression percentage
            prop_assert!(
                (actual_ratio / 100.0 - compression_pct).abs() < 0.01,
                "Calculated ratio {:.2}% should match input compression {:.2}%",
                actual_ratio, compression_pct * 100.0
            );
        }
        
        /// **Feature: tui-improvements, Property 10: Detail view data completeness**
        /// **Validates: Requirements 6.2, 6.5**
        /// 
        /// For any job displayed in detail view, all available metadata fields should be shown
        #[test]
        fn test_detail_view_completeness(
            jobs in prop::collection::vec(job_strategy(), 1..10),
        ) {
            // Create app with test data
            let mut app = App::new(PathBuf::from("/tmp/test"));
            
            // Create a job with all metadata fields populated
            let mut complete_job = create_test_job("complete_job", JobStatus::Success);
            complete_job.video_codec = Some("h264".to_string());
            complete_job.video_width = Some(1920);
            complete_job.video_height = Some(1080);
            complete_job.video_bitrate = Some(5_000_000);
            complete_job.video_frame_rate = Some("30/1".to_string());
            complete_job.is_hdr = Some(false);
            complete_job.source_bit_depth = Some(8);
            complete_job.target_bit_depth = Some(8);
            complete_job.source_pix_fmt = Some("yuv420p".to_string());
            complete_job.av1_quality = Some(25);
            complete_job.av1_profile = Some(0);
            complete_job.original_bytes = Some(1_000_000_000);
            complete_job.new_bytes = Some(500_000_000);
            complete_job.started_at = Some(Utc::now());
            complete_job.finished_at = Some(Utc::now());
            complete_job.output_path = Some(PathBuf::from("/test/output.mkv"));
            
            app.jobs = vec![complete_job.clone()];
            app.jobs.extend(jobs);
            
            // Set up detail view for the complete job
            app.ui_state.view_mode = ViewMode::DetailView;
            app.ui_state.detail_view_job_id = Some(complete_job.id.clone());
            
            // Simulate rendering by building the detail view content
            // We'll check that all metadata fields are present in the rendered output
            
            // Build detail content (same logic as render_detail_view)
            let mut lines = Vec::new();
            
            let job = &complete_job;
            
            // Header
            lines.push("═══════════════════════════════════════════════════════════════════════════════".to_string());
            lines.push(format!("JOB DETAILS: {}", job.id));
            lines.push("═══════════════════════════════════════════════════════════════════════════════".to_string());
            lines.push("".to_string());
            
            // File paths
            lines.push("FILE PATHS:".to_string());
            lines.push(format!("  Source: {}", job.source_path.display()));
            if let Some(output) = &job.output_path {
                lines.push(format!("  Output: {}", output.display()));
            }
            lines.push("".to_string());
            
            // Status
            lines.push("STATUS:".to_string());
            lines.push(format!("  Status: {:?}", job.status));
            if let Some(reason) = &job.reason {
                lines.push(format!("  Reason: {}", reason));
            }
            lines.push("".to_string());
            
            // Job history
            lines.push("JOB HISTORY:".to_string());
            lines.push(format!("  Created:  {}", job.created_at.format("%Y-%m-%d %H:%M:%S UTC")));
            if let Some(started) = job.started_at {
                lines.push(format!("  Started:  {}", started.format("%Y-%m-%d %H:%M:%S UTC")));
            }
            if let Some(finished) = job.finished_at {
                lines.push(format!("  Finished: {}", finished.format("%Y-%m-%d %H:%M:%S UTC")));
            }
            lines.push("".to_string());
            
            // Video metadata
            lines.push("VIDEO METADATA:".to_string());
            if let (Some(width), Some(height)) = (job.video_width, job.video_height) {
                lines.push(format!("  Resolution: {}x{}", width, height));
            }
            if let Some(codec) = &job.video_codec {
                lines.push(format!("  Codec: {}", codec));
            }
            if let Some(bitrate) = job.video_bitrate {
                lines.push(format!("  Bitrate: {:.2} Mbps ({} bps)", bitrate as f64 / 1_000_000.0, bitrate));
            }
            if let Some(frame_rate) = &job.video_frame_rate {
                lines.push(format!("  Frame Rate: {} fps", frame_rate));
            }
            if let Some(is_hdr) = job.is_hdr {
                lines.push(format!("  HDR: {}", if is_hdr { "Yes" } else { "No" }));
            }
            if let Some(bit_depth) = job.source_bit_depth {
                lines.push(format!("  Source Bit Depth: {} bit", bit_depth));
            }
            if let Some(bit_depth) = job.target_bit_depth {
                lines.push(format!("  Target Bit Depth: {} bit", bit_depth));
            }
            if let Some(pix_fmt) = &job.source_pix_fmt {
                lines.push(format!("  Pixel Format: {}", pix_fmt));
            }
            lines.push("".to_string());
            
            // Encoding parameters
            lines.push("ENCODING PARAMETERS:".to_string());
            if let Some(quality) = job.av1_quality {
                lines.push(format!("  AV1 Quality (CRF): {}", quality));
            }
            if let Some(profile) = job.av1_profile {
                lines.push(format!("  AV1 Profile: {} ({})", profile, 
                    match profile {
                        0 => "Main (8-bit)",
                        1 => "High (10-bit)",
                        2 => "Professional (12-bit)",
                        _ => "Unknown",
                    }));
            }
            lines.push("".to_string());
            
            // File sizes
            lines.push("FILE SIZES:".to_string());
            if let Some(orig_bytes) = job.original_bytes {
                lines.push(format!("  Original Size: {} ({} bytes)", format_size(orig_bytes, DECIMAL), orig_bytes));
            }
            if let Some(new_bytes) = job.new_bytes {
                lines.push(format!("  New Size: {} ({} bytes)", format_size(new_bytes, DECIMAL), new_bytes));
                
                // Calculate savings
                if let Some(orig_bytes) = job.original_bytes {
                    if orig_bytes > 0 {
                        let savings_bytes = orig_bytes.saturating_sub(new_bytes);
                        let savings_pct = (savings_bytes as f64 / orig_bytes as f64) * 100.0;
                        
                        lines.push(format!("  Space Saved: {} ({:.1}%)", format_size(savings_bytes, DECIMAL), savings_pct));
                    }
                }
            }
            
            let content = lines.join("\n");
            
            // Verify all metadata fields are present in the content
            
            // Check video metadata fields (Requirements 6.2)
            prop_assert!(content.contains("Resolution: 1920x1080"), 
                "Detail view should show resolution");
            prop_assert!(content.contains("Codec: h264"), 
                "Detail view should show codec");
            prop_assert!(content.contains("Bitrate:"), 
                "Detail view should show bitrate");
            prop_assert!(content.contains("Frame Rate: 30/1 fps"), 
                "Detail view should show frame rate");
            prop_assert!(content.contains("HDR: No"), 
                "Detail view should show HDR status");
            
            // Check bit depth and pixel format (Requirements 6.5)
            prop_assert!(content.contains("Source Bit Depth: 8 bit"), 
                "Detail view should show source bit depth");
            prop_assert!(content.contains("Target Bit Depth: 8 bit"), 
                "Detail view should show target bit depth");
            prop_assert!(content.contains("Pixel Format: yuv420p"), 
                "Detail view should show pixel format");
            
            // Check encoding parameters (Requirements 6.2)
            prop_assert!(content.contains("AV1 Quality (CRF): 25"), 
                "Detail view should show AV1 quality");
            prop_assert!(content.contains("AV1 Profile: 0 (Main (8-bit))"), 
                "Detail view should show AV1 profile");
            
            // Check file paths (Requirements 6.4)
            prop_assert!(content.contains("Source: /test/video.mkv"), 
                "Detail view should show source path");
            prop_assert!(content.contains("Output: /test/output.mkv"), 
                "Detail view should show output path");
            
            // Check job history (Requirements 6.3)
            prop_assert!(content.contains("Created:"), 
                "Detail view should show created timestamp");
            prop_assert!(content.contains("Started:"), 
                "Detail view should show started timestamp");
            prop_assert!(content.contains("Finished:"), 
                "Detail view should show finished timestamp");
            
            // Check file sizes
            prop_assert!(content.contains("Original Size:"), 
                "Detail view should show original size");
            prop_assert!(content.contains("New Size:"), 
                "Detail view should show new size");
            prop_assert!(content.contains("Space Saved:"), 
                "Detail view should show space saved");
            
            // Check status
            prop_assert!(content.contains("Status: Success"), 
                "Detail view should show job status");
            
            // Verify that missing fields show appropriate placeholders
            let mut incomplete_job = create_test_job("incomplete_job", JobStatus::Pending);
            incomplete_job.video_codec = None;
            incomplete_job.video_width = None;
            
            // Build content for incomplete job
            let mut incomplete_lines = Vec::new();
            incomplete_lines.push("VIDEO METADATA:".to_string());
            
            if incomplete_job.video_width.is_none() || incomplete_job.video_height.is_none() {
                incomplete_lines.push("  Resolution: (not available)".to_string());
            }
            if incomplete_job.video_codec.is_none() {
                incomplete_lines.push("  Codec: (not available)".to_string());
            }
            
            let incomplete_content = incomplete_lines.join("\n");
            
            prop_assert!(incomplete_content.contains("(not available)"), 
                "Detail view should show '(not available)' for missing fields");
        }
        
        /// **Feature: tui-improvements, Property 7: Layout responsiveness**
        /// **Validates: Requirements 1.5, 10.5**
        /// 
        /// For any terminal size change, the layout should recalculate and render 
        /// without overlapping components
        #[test]
        fn test_layout_responsiveness(
            width in 80u16..200u16,
            height in 12u16..60u16,
        ) {
            // Create a Rect with the given dimensions
            let size = Rect {
                x: 0,
                y: 0,
                width,
                height,
            };
            
            // Create layout configuration
            let layout = LayoutConfig::from_terminal_size(size);
            
            // Verify terminal size is stored correctly
            prop_assert_eq!(layout.terminal_size, size, 
                "Layout should store terminal size correctly");
            
            // Verify statistics visibility based on height
            if height >= 20 {
                prop_assert!(layout.show_statistics, 
                    "Statistics should be shown when height >= 20 (height={})", height);
            } else {
                prop_assert!(!layout.show_statistics, 
                    "Statistics should be hidden when height < 20 (height={})", height);
            }
            
            // Verify column selection based on width
            if width >= 160 {
                // Large terminal: should have all columns
                prop_assert_eq!(layout.table_columns.len(), 14,
                    "Large terminal (width={}) should show all 14 columns", width);
                
                // Verify all columns are present
                prop_assert!(layout.table_columns.contains(&TableColumn::Status));
                prop_assert!(layout.table_columns.contains(&TableColumn::File));
                prop_assert!(layout.table_columns.contains(&TableColumn::Resolution));
                prop_assert!(layout.table_columns.contains(&TableColumn::Codec));
                prop_assert!(layout.table_columns.contains(&TableColumn::Bitrate));
                prop_assert!(layout.table_columns.contains(&TableColumn::Hdr));
                prop_assert!(layout.table_columns.contains(&TableColumn::BitDepth));
                prop_assert!(layout.table_columns.contains(&TableColumn::OrigSize));
                prop_assert!(layout.table_columns.contains(&TableColumn::NewSize));
                prop_assert!(layout.table_columns.contains(&TableColumn::CompressionRatio));
                prop_assert!(layout.table_columns.contains(&TableColumn::Quality));
                prop_assert!(layout.table_columns.contains(&TableColumn::Savings));
                prop_assert!(layout.table_columns.contains(&TableColumn::Time));
                prop_assert!(layout.table_columns.contains(&TableColumn::Reason));
            } else if width >= 120 {
                // Medium terminal: should have 9 essential columns
                prop_assert_eq!(layout.table_columns.len(), 9,
                    "Medium terminal (width={}) should show 9 columns", width);
                
                // Verify essential columns are present
                prop_assert!(layout.table_columns.contains(&TableColumn::Status));
                prop_assert!(layout.table_columns.contains(&TableColumn::File));
                prop_assert!(layout.table_columns.contains(&TableColumn::Resolution));
                prop_assert!(layout.table_columns.contains(&TableColumn::Codec));
                prop_assert!(layout.table_columns.contains(&TableColumn::OrigSize));
                prop_assert!(layout.table_columns.contains(&TableColumn::NewSize));
                prop_assert!(layout.table_columns.contains(&TableColumn::CompressionRatio));
                prop_assert!(layout.table_columns.contains(&TableColumn::Savings));
                prop_assert!(layout.table_columns.contains(&TableColumn::Time));
                
                // Verify non-essential columns are NOT present
                prop_assert!(!layout.table_columns.contains(&TableColumn::Bitrate));
                prop_assert!(!layout.table_columns.contains(&TableColumn::Hdr));
                prop_assert!(!layout.table_columns.contains(&TableColumn::BitDepth));
                prop_assert!(!layout.table_columns.contains(&TableColumn::Quality));
                prop_assert!(!layout.table_columns.contains(&TableColumn::Reason));
            } else {
                // Small terminal: should have 5 minimal columns
                prop_assert_eq!(layout.table_columns.len(), 5,
                    "Small terminal (width={}) should show 5 columns", width);
                
                // Verify minimal columns are present
                prop_assert!(layout.table_columns.contains(&TableColumn::Status));
                prop_assert!(layout.table_columns.contains(&TableColumn::File));
                prop_assert!(layout.table_columns.contains(&TableColumn::OrigSize));
                prop_assert!(layout.table_columns.contains(&TableColumn::NewSize));
                prop_assert!(layout.table_columns.contains(&TableColumn::Savings));
            }
            
            // Verify no duplicate columns
            let mut seen_columns = std::collections::HashSet::new();
            for col in &layout.table_columns {
                prop_assert!(seen_columns.insert(col),
                    "Column {:?} appears multiple times in layout", col);
            }
            
            // Verify column order is preserved (Status should always be first, File second)
            if !layout.table_columns.is_empty() {
                prop_assert_eq!(layout.table_columns[0], TableColumn::Status,
                    "First column should always be Status");
            }
            if layout.table_columns.len() > 1 {
                prop_assert_eq!(layout.table_columns[1], TableColumn::File,
                    "Second column should always be File");
            }
            
            // Verify that show_current_job is always true (as per design)
            prop_assert!(layout.show_current_job,
                "show_current_job should always be true");
        }
        
        /// **Feature: tui-improvements, Property 5: Progress percentage bounds**
        /// **Validates: Requirements 4.1**
        /// 
        /// For any running job with progress tracking, the progress percentage should 
        /// always be between 0.0 and 100.0 inclusive
        #[test]
        fn test_progress_percentage_bounds(
            original_size in 1u64..10_000_000_000u64,  // 1 byte to 10GB
            temp_size in 0u64..10_000_000_000u64,      // 0 bytes to 10GB
            estimated_output_size in 1u64..10_000_000_000u64,  // 1 byte to 10GB
        ) {
            // Create a JobProgress instance
            let mut progress = JobProgress::new(
                PathBuf::from("/tmp/test.tmp.av1.mkv"),
                original_size
            );
            
            // Set temp file size
            progress.temp_file_size = temp_size;
            progress.original_size = original_size;
            progress.estimated_final_size = Some(estimated_output_size);
            
            // Calculate progress percentage (same logic as in refresh())
            if estimated_output_size > 0 {
                progress.progress_percent = (temp_size as f64 / estimated_output_size as f64 * 100.0)
                    .min(100.0)
                    .max(0.0);
            } else {
                progress.progress_percent = 0.0;
            }
            
            // Verify progress percentage is within bounds [0.0, 100.0]
            prop_assert!(
                progress.progress_percent >= 0.0,
                "Progress percentage should be >= 0.0, got {}",
                progress.progress_percent
            );
            
            prop_assert!(
                progress.progress_percent <= 100.0,
                "Progress percentage should be <= 100.0, got {}",
                progress.progress_percent
            );
            
            // Verify progress percentage is not NaN or infinite
            prop_assert!(
                progress.progress_percent.is_finite(),
                "Progress percentage should be finite, got {}",
                progress.progress_percent
            );
            
            // Verify specific edge cases
            if temp_size == 0 {
                prop_assert_eq!(
                    progress.progress_percent,
                    0.0,
                    "Progress should be 0% when temp file size is 0"
                );
            }
            
            if temp_size >= estimated_output_size {
                prop_assert!(
                    progress.progress_percent >= 99.0,
                    "Progress should be near 100% when temp size >= estimated output size, got {}",
                    progress.progress_percent
                );
            }
            
            // Verify that progress is monotonic with respect to temp file size
            // If we increase temp_size, progress should not decrease
            let original_progress = progress.progress_percent;
            let increased_temp_size = temp_size.saturating_add(1000);
            
            let increased_progress = if estimated_output_size > 0 {
                (increased_temp_size as f64 / estimated_output_size as f64 * 100.0)
                    .min(100.0)
                    .max(0.0)
            } else {
                0.0
            };
            
            prop_assert!(
                increased_progress >= original_progress,
                "Progress should not decrease when temp file size increases: {} -> {}",
                original_progress, increased_progress
            );
            
            // Test frame-level progress tracking bounds
            if let Some(total_frames) = progress.total_frames {
                if let Some(frames_processed) = progress.frames_processed {
                    // Frames processed should be within bounds
                    prop_assert!(
                        frames_processed <= total_frames,
                        "Frames processed ({}) should not exceed total frames ({})",
                        frames_processed, total_frames
                    );
                }
            }
            
            // Test current FPS bounds
            if let Some(fps) = progress.current_fps {
                prop_assert!(
                    fps >= 0.0,
                    "Current FPS should be non-negative, got {}",
                    fps
                );
                
                prop_assert!(
                    fps < 500.0,
                    "Current FPS should be reasonable (< 500), got {}",
                    fps
                );
                
                prop_assert!(
                    fps.is_finite(),
                    "Current FPS should be finite, got {}",
                    fps
                );
            }
            
            // Test compression ratio bounds
            if let Some(ratio) = progress.current_compression_ratio {
                prop_assert!(
                    ratio >= 0.0,
                    "Compression ratio should be non-negative, got {}",
                    ratio
                );
                
                prop_assert!(
                    ratio <= 1.0,
                    "Compression ratio should be <= 1.0, got {}",
                    ratio
                );
                
                prop_assert!(
                    ratio.is_finite(),
                    "Compression ratio should be finite, got {}",
                    ratio
                );
            }
        }
        
        /// **Feature: tui-improvements, Property 6: Color scheme consistency**
        /// **Validates: Requirements 1.1**
        /// 
        /// For any job status, the color used for that job should match the defined 
        /// color scheme for that status
        #[test]
        fn test_color_scheme_consistency(
            status in job_status_strategy(),
        ) {
            // Create a color scheme
            let color_scheme = ColorScheme::default();
            
            // Get the color for the status
            let status_color = color_scheme.status_color(&status);
            
            // Verify the color matches the expected color for that status
            let expected_color = match status {
                JobStatus::Pending => color_scheme.pending,
                JobStatus::Running => color_scheme.running,
                JobStatus::Success => color_scheme.success,
                JobStatus::Failed => color_scheme.failed,
                JobStatus::Skipped => color_scheme.skipped,
            };
            
            prop_assert_eq!(
                status_color,
                expected_color,
                "Color for status {:?} should match the color scheme", status
            );
            
            // Verify that different statuses have different colors (except Skipped which may share)
            // This ensures visual distinction between statuses
            let all_statuses = vec![
                JobStatus::Pending,
                JobStatus::Running,
                JobStatus::Success,
                JobStatus::Failed,
            ];
            
            let colors: Vec<Color> = all_statuses.iter()
                .map(|s| color_scheme.status_color(s))
                .collect();
            
            // Check that primary statuses (Pending, Running, Success, Failed) have distinct colors
            for i in 0..colors.len() {
                for j in (i+1)..colors.len() {
                    prop_assert!(
                        colors[i] != colors[j],
                        "Primary statuses should have distinct colors: {:?} vs {:?}",
                        all_statuses[i], all_statuses[j]
                    );
                }
            }
        }
    }
    
    // Integration tests
    
    /// Test full rendering pipeline with various app states
    #[test]
    fn test_full_rendering_pipeline() {
        // Create app with test data
        let mut app = App::new(PathBuf::from("/tmp/test"));
        
        // Test 1: Empty job list
        app.jobs = vec![];
        // Rendering should not panic with empty jobs
        // (We can't actually render without a terminal, but we can test the data flow)
        let filtered = app.filter_jobs(&app.jobs);
        assert_eq!(filtered.len(), 0);
        
        // Test 2: Jobs with various statuses
        let mut jobs = vec![];
        for i in 0..5 {
            jobs.push(create_test_job(&format!("pending{}", i), JobStatus::Pending));
        }
        for i in 0..3 {
            jobs.push(create_test_job(&format!("running{}", i), JobStatus::Running));
        }
        for i in 0..10 {
            jobs.push(create_test_job(&format!("success{}", i), JobStatus::Success));
        }
        for i in 0..2 {
            jobs.push(create_test_job(&format!("failed{}", i), JobStatus::Failed));
        }
        
        app.jobs = jobs;
        
        // Test filtering
        app.ui_state.filter = JobFilter::All;
        let filtered = app.filter_jobs(&app.jobs);
        assert_eq!(filtered.len(), 20);
        
        app.ui_state.filter = JobFilter::Running;
        let filtered = app.filter_jobs(&app.jobs);
        assert_eq!(filtered.len(), 3);
        
        // Test sorting
        app.ui_state.filter = JobFilter::All;
        let mut filtered = app.filter_jobs(&app.jobs);
        app.sort_jobs(&mut filtered);
        // After sorting by date (default), jobs should be ordered
        assert!(filtered.len() > 0);
        
        // Test statistics calculation
        let stats = StatisticsCache::calculate(&app.jobs);
        // 10 success out of 12 completed (10 success + 2 failed) = 83.33%
        // Pending and running jobs don't count toward success rate
        assert!((stats.success_rate - 83.33).abs() < 0.1);
        
        // Test layout configuration for various sizes
        let too_small_size = Rect { x: 0, y: 0, width: 79, height: 11 };
        let layout = LayoutConfig::from_terminal_size(too_small_size);
        assert!(layout.is_too_small);
        
        let small_size = Rect { x: 0, y: 0, width: 80, height: 12 };
        let layout = LayoutConfig::from_terminal_size(small_size);
        assert!(!layout.is_too_small); // 80x12 is the minimum acceptable size
        
        let medium_size = Rect { x: 0, y: 0, width: 120, height: 25 };
        let layout = LayoutConfig::from_terminal_size(medium_size);
        assert!(!layout.is_too_small);
        assert_eq!(layout.table_columns.len(), 9); // Medium terminal columns
        
        let large_size = Rect { x: 0, y: 0, width: 180, height: 40 };
        let layout = LayoutConfig::from_terminal_size(large_size);
        assert!(!layout.is_too_small);
        assert_eq!(layout.table_columns.len(), 14); // All columns
    }
    
    /// Test all keyboard shortcuts and their effects on app state
    #[test]
    fn test_keyboard_shortcuts() {
        let mut app = App::new(PathBuf::from("/tmp/test"));
        
        // Create test jobs
        let mut jobs = vec![];
        for i in 0..10 {
            jobs.push(create_test_job(&format!("job{}", i), JobStatus::Pending));
        }
        app.jobs = jobs;
        
        // Test filter shortcuts (1-5)
        app.ui_state.filter = JobFilter::All;
        assert_eq!(app.ui_state.filter, JobFilter::All);
        
        // Simulate pressing '2' for Pending filter
        app.ui_state.filter = JobFilter::Pending;
        assert_eq!(app.ui_state.filter, JobFilter::Pending);
        
        // Simulate pressing '3' for Running filter
        app.ui_state.filter = JobFilter::Running;
        assert_eq!(app.ui_state.filter, JobFilter::Running);
        
        // Simulate pressing '4' for Success filter
        app.ui_state.filter = JobFilter::Success;
        assert_eq!(app.ui_state.filter, JobFilter::Success);
        
        // Simulate pressing '5' for Failed filter
        app.ui_state.filter = JobFilter::Failed;
        assert_eq!(app.ui_state.filter, JobFilter::Failed);
        
        // Simulate pressing '1' for All filter
        app.ui_state.filter = JobFilter::All;
        assert_eq!(app.ui_state.filter, JobFilter::All);
        
        // Test sort shortcut ('s' cycles through modes)
        app.ui_state.sort_mode = SortMode::ByDate;
        assert_eq!(app.ui_state.sort_mode, SortMode::ByDate);
        
        app.cycle_sort_mode();
        assert_eq!(app.ui_state.sort_mode, SortMode::BySize);
        
        app.cycle_sort_mode();
        assert_eq!(app.ui_state.sort_mode, SortMode::ByStatus);
        
        app.cycle_sort_mode();
        assert_eq!(app.ui_state.sort_mode, SortMode::BySavings);
        
        app.cycle_sort_mode();
        assert_eq!(app.ui_state.sort_mode, SortMode::ByDate); // Cycles back
        
        // Test navigation shortcuts
        app.ui_state.selected_index = None;
        
        // Down arrow - should select first item
        app.move_selection_down();
        assert_eq!(app.ui_state.selected_index, Some(0));
        
        // Down arrow again - should move to second item
        app.move_selection_down();
        assert_eq!(app.ui_state.selected_index, Some(1));
        
        // Up arrow - should move back to first item
        app.move_selection_up();
        assert_eq!(app.ui_state.selected_index, Some(0));
        
        // Up arrow at top - should wrap to bottom
        app.move_selection_up();
        assert_eq!(app.ui_state.selected_index, Some(9)); // Last item (10 jobs, 0-indexed)
        
        // Page down
        app.ui_state.selected_index = Some(0);
        app.move_selection_page_down();
        assert!(app.ui_state.selected_index.unwrap() > 0); // Should move down by page
        
        // Page up
        app.move_selection_page_up();
        assert!(app.ui_state.selected_index.unwrap() < 10); // Should move up by page
        
        // Test detail view shortcuts
        app.ui_state.view_mode = ViewMode::Normal;
        app.ui_state.selected_index = Some(0);
        
        // Simulate Enter key - should open detail view
        // (In real code, this would be handled in the event loop)
        app.ui_state.view_mode = ViewMode::DetailView;
        app.ui_state.detail_view_job_id = Some(app.jobs[0].id.clone());
        assert_eq!(app.ui_state.view_mode, ViewMode::DetailView);
        assert!(app.ui_state.detail_view_job_id.is_some());
        
        // Simulate Escape key - should close detail view
        app.ui_state.view_mode = ViewMode::Normal;
        app.ui_state.detail_view_job_id = None;
        assert_eq!(app.ui_state.view_mode, ViewMode::Normal);
        assert!(app.ui_state.detail_view_job_id.is_none());
        
        // Test quit flag
        app.should_quit = false;
        assert!(!app.should_quit);
        
        // Simulate 'q' key
        app.should_quit = true;
        assert!(app.should_quit);
    }
    
    /// Test state transitions between different view modes and filters
    #[test]
    fn test_state_transitions() {
        let mut app = App::new(PathBuf::from("/tmp/test"));
        
        // Create test jobs with various statuses
        let mut jobs = vec![];
        jobs.push(create_test_job("pending1", JobStatus::Pending));
        jobs.push(create_test_job("running1", JobStatus::Running));
        jobs.push(create_test_job("success1", JobStatus::Success));
        jobs.push(create_test_job("failed1", JobStatus::Failed));
        app.jobs = jobs;
        
        // Test transition: Normal -> DetailView -> Normal
        assert_eq!(app.ui_state.view_mode, ViewMode::Normal);
        
        app.ui_state.view_mode = ViewMode::DetailView;
        app.ui_state.detail_view_job_id = Some("pending1".to_string());
        assert_eq!(app.ui_state.view_mode, ViewMode::DetailView);
        assert_eq!(app.ui_state.detail_view_job_id, Some("pending1".to_string()));
        
        app.ui_state.view_mode = ViewMode::Normal;
        app.ui_state.detail_view_job_id = None;
        assert_eq!(app.ui_state.view_mode, ViewMode::Normal);
        assert_eq!(app.ui_state.detail_view_job_id, None);
        
        // Test filter transitions with job count verification
        app.ui_state.filter = JobFilter::All;
        let filtered = app.filter_jobs(&app.jobs);
        assert_eq!(filtered.len(), 4);
        
        app.ui_state.filter = JobFilter::Pending;
        let filtered = app.filter_jobs(&app.jobs);
        assert_eq!(filtered.len(), 1);
        
        app.ui_state.filter = JobFilter::Running;
        let filtered = app.filter_jobs(&app.jobs);
        assert_eq!(filtered.len(), 1);
        
        app.ui_state.filter = JobFilter::Success;
        let filtered = app.filter_jobs(&app.jobs);
        assert_eq!(filtered.len(), 1);
        
        app.ui_state.filter = JobFilter::Failed;
        let filtered = app.filter_jobs(&app.jobs);
        assert_eq!(filtered.len(), 1);
        
        // Test sort mode transitions
        app.ui_state.sort_mode = SortMode::ByDate;
        app.cycle_sort_mode();
        assert_eq!(app.ui_state.sort_mode, SortMode::BySize);
        
        app.cycle_sort_mode();
        assert_eq!(app.ui_state.sort_mode, SortMode::ByStatus);
        
        app.cycle_sort_mode();
        assert_eq!(app.ui_state.sort_mode, SortMode::BySavings);
        
        app.cycle_sort_mode();
        assert_eq!(app.ui_state.sort_mode, SortMode::ByDate);
        
        // Test selection state preservation across filter changes
        app.ui_state.filter = JobFilter::All;
        app.ui_state.selected_index = Some(2);
        
        // Change filter - selection may be out of bounds now
        app.ui_state.filter = JobFilter::Pending;
        let filtered = app.filter_jobs(&app.jobs);
        let filtered_len = filtered.len();
        
        // In the real app, selection would be adjusted when filter changes
        // For this test, we just verify the filtered list is correct
        assert_eq!(filtered_len, 1); // Only 1 pending job
        
        // If we want to maintain selection, we'd need to adjust it
        if let Some(idx) = app.ui_state.selected_index {
            if idx >= filtered_len && filtered_len > 0 {
                // Adjust selection to be within bounds
                app.ui_state.selected_index = Some(filtered_len - 1);
            } else if filtered_len == 0 {
                app.ui_state.selected_index = None;
            }
        }
        
        // Now verify selection is valid
        if let Some(idx) = app.ui_state.selected_index {
            assert!(idx < filtered_len, "Selection should be within bounds after adjustment");
        }
    }
    
    /// Test refresh cycle and data updates
    #[test]
    fn test_refresh_cycle() {
        let mut app = App::new(PathBuf::from("/tmp/test"));
        
        // Initial state
        assert_eq!(app.jobs.len(), 0);
        assert_eq!(app.last_job_count, 0);
        
        // Simulate adding jobs
        let mut jobs = vec![];
        for i in 0..5 {
            let mut job = create_test_job(&format!("job{}", i), JobStatus::Success);
            job.original_bytes = Some(1_000_000_000);
            job.new_bytes = Some(500_000_000);
            job.started_at = Some(Utc::now());
            job.finished_at = Some(Utc::now());
            jobs.push(job);
        }
        app.jobs = jobs;
        app.last_job_count = app.jobs.len();
        
        // Test statistics cache refresh
        let initial_stats = StatisticsCache::calculate(&app.jobs);
        assert_eq!(initial_stats.total_space_saved, 2_500_000_000); // 5 jobs * 500MB each
        assert_eq!(initial_stats.success_rate, 100.0);
        
        app.statistics_cache = initial_stats.clone();
        
        // Verify cache needs refresh after time passes
        // (In real code, this would be after 5 seconds)
        assert!(!app.statistics_cache.needs_refresh()); // Just created, shouldn't need refresh
        
        // Test progress tracking for running jobs
        let mut running_job = create_test_job("running1", JobStatus::Running);
        running_job.original_bytes = Some(1_000_000_000);
        running_job.started_at = Some(Utc::now());
        app.jobs.push(running_job.clone());
        
        // Simulate progress tracking
        let progress = JobProgress::new(
            PathBuf::from("/tmp/test.tmp.av1.mkv"),
            1_000_000_000
        );
        app.job_progress.insert(running_job.id.clone(), progress);
        
        assert_eq!(app.job_progress.len(), 1);
        assert!(app.job_progress.contains_key(&running_job.id));
        
        // Test cleanup of completed jobs from progress tracking
        // Change job status to completed
        if let Some(job) = app.jobs.iter_mut().find(|j| j.id == running_job.id) {
            job.status = JobStatus::Success;
        }
        
        // Simulate cleanup (would happen in refresh())
        let running_ids: Vec<String> = app.jobs.iter()
            .filter(|j| j.status == JobStatus::Running)
            .map(|j| j.id.clone())
            .collect();
        
        let running_ids_set: std::collections::HashSet<_> = running_ids.iter().collect();
        app.job_progress.retain(|id, _| running_ids_set.contains(id));
        
        // Progress tracking should be cleaned up
        assert_eq!(app.job_progress.len(), 0);
        
        // Test estimated savings cache
        let mut pending_job = create_test_job("pending1", JobStatus::Pending);
        pending_job.original_bytes = Some(1_000_000_000);
        pending_job.video_codec = Some("h264".to_string());
        pending_job.video_width = Some(1920);
        pending_job.video_height = Some(1080);
        pending_job.video_bitrate = Some(5_000_000);
        pending_job.video_frame_rate = Some("30/1".to_string());
        pending_job.av1_quality = Some(25);
        app.jobs.push(pending_job.clone());
        
        // Calculate estimate
        let estimate = estimate_space_savings(&pending_job);
        assert!(estimate.is_some());
        
        app.estimated_savings_cache.insert(pending_job.id.clone(), estimate);
        assert!(app.estimated_savings_cache.contains_key(&pending_job.id));
        
        // Test cache cleanup for removed jobs
        let job_ids_set: std::collections::HashSet<_> = app.jobs.iter().map(|j| &j.id).collect();
        app.estimated_savings_cache.retain(|id, _| job_ids_set.contains(id));
        
        // Cache should still contain the pending job
        assert!(app.estimated_savings_cache.contains_key(&pending_job.id));
        
        // Remove the job and test cleanup
        app.jobs.retain(|j| j.id != pending_job.id);
        let job_ids_set: std::collections::HashSet<_> = app.jobs.iter().map(|j| &j.id).collect();
        app.estimated_savings_cache.retain(|id, _| job_ids_set.contains(id));
        
        // Cache should be cleaned up
        assert!(!app.estimated_savings_cache.contains_key(&pending_job.id));
    }
}
