use anyhow::{Context, Result};
use clap::Parser;
use chrono::{Utc, DateTime};
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

/// Estimate space savings in GB for AV1 transcoding based on video properties
/// Uses bitrate estimation considering resolution, frame rate, and source codec efficiency
/// Returns None if required metadata is not available
fn estimate_space_savings_gb(job: &Job) -> Option<f64> {
    let orig_bytes = job.original_bytes?;
    if orig_bytes == 0 {
        return None;
    }
    
    // Require all metadata for proper estimation - no lazy fallbacks
    // We validate these are present even if not directly used in calculation
    // (they ensure we have complete video metadata)
    let (_width, _height, bitrate_bps, codec, frame_rate_str) = (
        job.video_width?,
        job.video_height?,
        job.video_bitrate?,
        job.video_codec.as_deref()?,
        job.video_frame_rate.as_deref()?,
    );
    
    // Parse frame rate to validate it's valid (format: "30/1" or "29.97")
    // We require it to be present and parseable, even if not used in calculation
    let _fps = parse_frame_rate(frame_rate_str)?;
    
    // Adjust based on source codec efficiency
    // AV1 efficiency vs source codec (bitrate reduction factor)
    // Based on real-world AV1 encoding studies and benchmarks
    let codec_efficiency_factor = match codec.to_lowercase().as_str() {
        "hevc" | "h265" => 0.55,  // AV1 uses ~55% of HEVC bitrate (45% reduction)
        "h264" | "avc" => 0.40,   // AV1 uses ~40% of H.264 bitrate (60% reduction)
        "vp9" => 0.85,            // AV1 uses ~85% of VP9 bitrate (15% reduction)
        "av1" => 1.0,             // Already AV1, no savings
        _ => return None,         // Unknown codec - cannot estimate without knowing efficiency
    };
    
    // Use source bitrate for accurate estimation
    let source_bitrate_mbps = bitrate_bps as f64 / 1_000_000.0;
    if source_bitrate_mbps <= 0.0 {
        return None; // Invalid bitrate
    }
    
    // Calculate AV1 bitrate using source bitrate adjusted by codec efficiency
    let estimated_av1_bitrate_mbps = source_bitrate_mbps * codec_efficiency_factor;
    
    // Calculate duration from file size and bitrate
    // Total bitrate includes video + audio + overhead
    // Estimate audio bitrate based on typical audio encoding (128-256 kbps average)
    let estimated_audio_bitrate_mbps = 0.2; // 200 kbps average
    let total_source_bitrate_mbps = source_bitrate_mbps + estimated_audio_bitrate_mbps;
    
    // Calculate duration in seconds: duration = (file_size_bytes * 8) / bitrate_bps
    let duration_seconds = (orig_bytes as f64 * 8.0) / (total_source_bitrate_mbps * 1_000_000.0);
    if duration_seconds <= 0.0 {
        return None; // Invalid duration
    }
    
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
    jobs: Vec<Job>,
    system: System,
    table_state: TableState,
    should_quit: bool,
    job_state_dir: PathBuf,
    command_dir: PathBuf,  // Directory for writing command files
    job_progress: HashMap<String, JobProgress>,  // Track progress for running jobs
    last_refresh: DateTime<Utc>,  // Last refresh timestamp
    last_job_count: usize,  // Track job count changes for activity detection
    last_message: Option<String>,  // Status message to display
    message_timeout: Option<DateTime<Utc>>,  // When to clear message
}

impl App {
    fn new(job_state_dir: PathBuf) -> Self {
        // Derive command_dir from job_state_dir
        let command_dir = job_state_dir.parent()
            .map(|p| p.join("commands"))
            .unwrap_or_else(|| PathBuf::from("/var/lib/av1d/commands"));
        
        Self {
            jobs: Vec::new(),
            system: System::new(),
            table_state: TableState::default(),
            should_quit: false,
            job_state_dir,
            command_dir,
            job_progress: HashMap::new(),
            last_refresh: Utc::now(),
            last_job_count: 0,
            last_message: None,
            message_timeout: None,
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
        let temp_output = job.source_path.with_extension("tmp.av1.mkv");
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
            
            // Estimate output size based on codec efficiency
            let estimated_output_size = if let Some(codec) = &job.video_codec {
                let efficiency_factor = match codec.to_lowercase().as_str() {
                    "hevc" | "h265" => 0.55,
                    "h264" | "avc" => 0.40,
                    "vp9" => 0.85,
                    "av1" => 1.0,
                    _ => 0.5, // Default conservative estimate
                };
                (original_size as f64 * efficiency_factor) as u64
            } else {
                (original_size as f64 * 0.5) as u64 // Default 50% if codec unknown
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
                let temp_output = source_path.with_extension("tmp.av1.mkv");
                let original_size = original_bytes.unwrap_or(0);
                
                // Check if temp file exists and update progress
                if let Ok(metadata) = std::fs::metadata(&temp_output) {
                    let current_temp_size = metadata.len();
                    
                    // Get or create progress tracking
                    let mut progress = self.job_progress.get(job_id)
                        .cloned()
                        .unwrap_or_else(|| JobProgress::new(temp_output.clone(), original_size));
                    
                    // Calculate bytes per second
                    let time_delta_seconds = (now - progress.last_updated).num_seconds().max(1) as f64;
                    if time_delta_seconds > 0.0 && current_temp_size > progress.temp_file_size {
                        let bytes_delta = current_temp_size - progress.temp_file_size;
                        progress.bytes_per_second = bytes_delta as f64 / time_delta_seconds;
                    }
                    
                    // Update progress tracking
                    progress.temp_file_size = current_temp_size;
                    progress.last_updated = now;
                    
                    // Estimate output size
                    let estimated_output_size = if let Some(codec) = &video_codec {
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
        
        self.last_refresh = now;
        
        Ok(())
    }
    
    /// Get activity status based on job state changes and running jobs
    fn get_activity_status(&self) -> (&'static str, Color) {
        let running_count = self.jobs.iter()
            .filter(|j| j.status == JobStatus::Running)
            .count();
        
        if running_count > 0 {
            ("⚙️  Processing", Color::Green)
        } else {
            let pending_count = self.jobs.iter()
                .filter(|j| j.status == JobStatus::Pending)
                .count();
            
            if pending_count > 0 {
                ("💤 Idle", Color::Yellow)
            } else {
                ("💤 Idle", Color::Blue)
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
    let mut app = App::new(cfg.job_state_dir.clone());

    // Main event loop with adaptive refresh rate
    loop {
        // Check if there's an active job to determine refresh rate
        let has_active_job = app.jobs.iter().any(|j| j.status == JobStatus::Running);
        
        // Refresh data
        app.refresh()?;

        // Draw UI
        terminal.draw(|f| ui(f, &mut app))?;

        // Handle input with adaptive timeout
        // Faster refresh (1s) when active, slower (5s) when idle
        let poll_timeout = if has_active_job {
            Duration::from_millis(1000)  // 1 second when active
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
    let current_job_height = if running_job.is_some() { 6 } else { 0 }; // 3 lines of text + 1 progress bar + 2 borders
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
        
        // Split area into text and progress bar
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(3),  // Text info
                Constraint::Length(1),  // Progress bar
            ])
            .split(area);
        
        // Build info text
        let info_lines = vec![
            format!("STAGE: {} | FILE: {}", stage_str, truncate_string(&file_name, 50)),
            format!("ORIG: {} | CURRENT: {} | PROGRESS: {:.1}% | ETA: {}", orig_size, new_size, progress_pct, eta_str),
            format!("SPEED: {} | ELAPSED: {}", speed_str, duration),
        ];
        
        let paragraph = Paragraph::new(info_lines.join("\n"))
            .block(Block::default()
                .borders(Borders::ALL)
                .title("▶ CURRENT JOB")
                .style(Style::default().fg(Color::Green)))
            .style(Style::default().fg(Color::Yellow));
        f.render_widget(paragraph, chunks[0]);
        
        // Render progress bar
        let progress_color = match stage_str {
            "Transcoding" => Color::Green,
            "Probing" | "Verifying" => Color::Yellow,
            _ => Color::Cyan,
        };
        
        let progress_percent_u16 = progress_pct.min(100.0).max(0.0) as u16;
        let progress_gauge = Gauge::default()
            .block(Block::default()
                .borders(Borders::NONE)
                .title(format!("Progress: {:.1}%", progress_pct)))
            .gauge_style(Style::default().fg(progress_color))
            .percent(progress_percent_u16)
            .label(format!("{:.1}%", progress_pct));
        f.render_widget(progress_gauge, chunks[1]);
    }
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
    
    // Render activity status
    let (activity_text, activity_color) = app.get_activity_status();
    let activity_block = Block::default()
        .borders(Borders::ALL)
        .title("Status")
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

    // CPU gauge
    let cpu_percent_u16 = cpu_usage.min(100.0).max(0.0) as u16;
    let cpu_gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title("CPU"))
        .gauge_style(Style::default().fg(Color::Cyan))
        .percent(cpu_percent_u16)
        .label(format!("{:.1}%", cpu_usage));
    f.render_widget(cpu_gauge, gauge_chunks[0]);

    // Memory gauge
    let memory_percent_u16 = memory_percent.min(100.0).max(0.0) as u16;
    let memory_gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title("Memory"))
        .gauge_style(Style::default().fg(Color::Green))
        .percent(memory_percent_u16)
        .label(format!("{:.1}%", memory_percent));
    f.render_widget(memory_gauge, gauge_chunks[1]);

    // GPU gauge
    let gpu_usage = app.get_gpu_usage();
    let gpu_percent_u16 = gpu_usage.min(100.0).max(0.0) as u16;
    let gpu_gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title("GPU"))
        .gauge_style(Style::default().fg(Color::Magenta))
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
                    if has_estimation_metadata(job) {
                        if let Some(savings_gb) = estimate_space_savings_gb(job) {
                            format!("~{:.1}GB", savings_gb)
                        } else {
                            // Calculation failed despite having metadata - show why
                            let missing = vec![
                                if job.original_bytes.is_none() { "orig_bytes" } else { "" },
                                if job.video_codec.is_none() { "codec" } else { "" },
                                if job.video_width.is_none() { "width" } else { "" },
                                if job.video_height.is_none() { "height" } else { "" },
                                if job.video_bitrate.is_none() { "bitrate" } else { "" },
                                if job.video_frame_rate.is_none() { "fps" } else { "" },
                            ].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(",");
                            if missing.is_empty() {
                                "calc?".to_string() // All metadata present but calc failed
                            } else {
                                format!("no:{}", truncate_string(&missing, 8))
                            }
                        }
                    } else {
                        // Show what's missing
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

    // Include message if present
    let message_part = if let Some(msg) = &app.last_message {
        format!(" | MSG: {}", msg)
    } else {
        String::new()
    };
    
    let status_text = format!(
        "Total: {} | Running: {} (max 1) | Pending: {} | Success: {} | Failed: {} | Skipped: {} | Dir: {}{} | q=quit r=refresh R=requeue",
        total, running, pending, success, failed, skipped, dir_short, message_part
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

