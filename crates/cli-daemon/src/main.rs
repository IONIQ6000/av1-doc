use anyhow::{Context, Result};
use clap::Parser;
use daemon::{
    config::TranscodeConfig, 
    job::{self, Job, JobStatus, load_all_jobs, save_job},
    scan, ffprobe, classifier, sidecar,
    FFmpegManager, CommandBuilder,
    quality::QualityCalculator,
    classifier::QualityTier,
    test_clip::TestClipWorkflow,
};
use std::path::{Path, PathBuf};
use std::fs;
use std::collections::{HashMap, HashSet};
use chrono::{Utc, DateTime};
use log::{info, warn, error, debug};

/// Generate temp output path using configured temp_output_dir
fn get_temp_output_path(cfg: &TranscodeConfig, source_path: &Path) -> PathBuf {
    // Always use configured temp directory with source filename
    let filename = source_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");
    cfg.temp_output_dir.join(format!("{}.tmp.av1.mkv", filename))
}

/// AV1 transcoding daemon
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to configuration file (JSON or TOML)
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logger - use RUST_LOG env var or default to info level
    env_logger::Builder::from_default_env()
        .format_timestamp_secs()
        .init();

    let args = Args::parse();

    // Load configuration
    let cfg = TranscodeConfig::load_config(args.config.as_deref())
        .context("Failed to load configuration")?;

    info!("AV1 Daemon starting");
    info!("Configuration loaded:");
    info!("  Library roots: {:?}", cfg.library_roots);
    info!("  Min bytes: {}", cfg.min_bytes);
    info!("  Max size ratio: {}", cfg.max_size_ratio);
    info!("  Job state dir: {}", cfg.job_state_dir.display());
    info!("  Scan interval: {}s", cfg.scan_interval_secs);
    
    // Initialize FFmpeg Manager
    info!("Initializing FFmpeg Manager...");
    let ffmpeg_mgr = FFmpegManager::new(&cfg).await
        .context("Failed to initialize FFmpeg Manager - ensure FFmpeg 8.0+ is installed with AV1 encoder support")?;
    
    info!("✅ FFmpeg Manager initialized successfully");
    info!("  FFmpeg version: {}.{}.{}", 
          ffmpeg_mgr.version.major, 
          ffmpeg_mgr.version.minor, 
          ffmpeg_mgr.version.patch);
    info!("  Selected encoder: {:?}", ffmpeg_mgr.best_encoder());
    
    // Verify library roots exist
    for root in &cfg.library_roots {
        if root.exists() {
            info!("Library root exists: {}", root.display());
        } else {
            warn!("Library root does not exist: {}", root.display());
        }
    }

    // Ensure job state directory exists
    fs::create_dir_all(&cfg.job_state_dir)
        .with_context(|| format!("Failed to create job state directory: {}", cfg.job_state_dir.display()))?;

    // Recovery on startup: check for stuck jobs and orphaned temp files
    info!("🔄 Starting recovery checks...");
    let recovered_count = recover_stuck_jobs(&cfg).await
        .context("Failed to recover stuck jobs on startup")?;
    let cleaned_count = cleanup_orphaned_temp_files(&cfg).await
        .context("Failed to cleanup orphaned temp files on startup")?;
    if recovered_count > 0 || cleaned_count > 0 {
        info!("✅ Startup recovery complete: {} job(s) recovered, {} temp file(s) cleaned", 
              recovered_count, cleaned_count);
    } else {
        info!("✅ Startup recovery complete: no stuck jobs or orphaned files found");
    }

    // Main daemon loop
    let mut scan_count = 0u64;
    loop {
        scan_count += 1;
        info!("Starting library scan #{}", scan_count);

        let scan_results = scan::scan_library(&cfg).await
            .context("Failed to scan library")?;

        info!("Scan completed: found {} results (candidates + skipped)", scan_results.len());
        
        let candidates_in_results: usize = scan_results.iter()
            .filter(|r| matches!(r, scan::ScanResult::Candidate(_, _)))
            .count();
        info!("Scan found {} candidates ready for processing", candidates_in_results);

        // Create jobs for new candidates
        let existing_jobs = load_all_jobs(&cfg.job_state_dir)
            .context("Failed to load existing jobs")?;

        info!("Loaded {} existing jobs", existing_jobs.len());

        let existing_paths: HashSet<_> = existing_jobs
            .iter()
            .map(|j| &j.source_path)
            .collect();

        let mut candidates_count = 0;
        let mut skipped_count = 0;
        let mut new_jobs_count = 0;

        for result in scan_results {
            match result {
                scan::ScanResult::Candidate(path, size) => {
                    candidates_count += 1;
                    if !existing_paths.contains(&path) {
                        let mut job = Job::new(path.clone());
                        job.original_bytes = Some(size);
                        save_job(&job, &cfg.job_state_dir)
                            .with_context(|| format!("Failed to save job for: {}", path.display()))?;

                        new_jobs_count += 1;
                        info!("Created job {} for: {} ({} bytes)", job.id, path.display(), size);
                    } else {
                        debug!("File already has a job: {}", path.display());
                    }
                }
                scan::ScanResult::Skipped(path, reason) => {
                    skipped_count += 1;
                    debug!("Skipped {}: {}", path.display(), reason);
                }
            }
        }

        info!("Scan summary: {} candidates, {} skipped, {} new jobs created", 
              candidates_count, skipped_count, new_jobs_count);

        // Process command files from TUI (e.g., manual requeue requests)
        let _processed_commands = process_command_files(&cfg).await
            .context("Failed to process command files")?;
        
        // Periodic stuck job check - recover any stuck jobs before processing
        let _recovered_count = recover_stuck_jobs(&cfg).await
            .context("Failed to check for stuck jobs")?;
        if _recovered_count > 0 {
            info!("⚠️  Recovered {} stuck job(s) during periodic check", _recovered_count);
        }

        // Process pending jobs
        let mut jobs = load_all_jobs(&cfg.job_state_dir)
            .context("Failed to load jobs")?;

        let pending_count = jobs.iter().filter(|j| j.status == JobStatus::Pending).count();
        let running_count = jobs.iter().filter(|j| j.status == JobStatus::Running).count();

        // Log job counts
        if pending_count > 0 || running_count > 0 {
            info!("Job status: {} pending, {} running (max 1 concurrent transcoding job)", pending_count, running_count);
        }

        // Extract metadata for pending jobs in background (for EST SAVE calculation in TUI)
        // This runs regardless of whether a job is currently transcoding
        // Process multiple jobs in parallel (ffprobe is lightweight)
        let pending_jobs_without_metadata: Vec<Job> = jobs.iter()
            .filter(|j| j.status == JobStatus::Pending)
            .filter(|j| {
                // Check if metadata is missing
                j.video_codec.is_none() || 
                j.video_width.is_none() || 
                j.video_height.is_none() || 
                j.video_bitrate.is_none() || 
                j.video_frame_rate.is_none()
            })
            .take(5) // Process up to 5 jobs per scan interval (ffprobe is lightweight)
            .cloned()
            .collect();
        
        if !pending_jobs_without_metadata.is_empty() {
            info!("📊 Extracting metadata for {} pending job(s) in background (for EST SAVE)...", pending_jobs_without_metadata.len());
            
            // Spawn background tasks for each job
            for job in pending_jobs_without_metadata {
                let cfg_clone = cfg.clone();
                let job_id = job.id.clone();
                let job_path = job.source_path.clone();
                let job_state_dir = cfg.job_state_dir.clone();
                
                tokio::spawn(async move {
                    if let Err(e) = extract_metadata_for_job(&cfg_clone, &job_id, &job_path, &job_state_dir).await {
                        warn!("Failed to extract metadata for pending job {}: {}", job_id, e);
                    }
                });
            }
        }

        // Only start a new job if no jobs are currently running
        // This ensures only one transcoding job runs at a time (important for single GPU)
        if running_count == 0 {
            // Find a pending job
            if let Some(job) = jobs.iter_mut().find(|j| j.status == JobStatus::Pending) {
                info!("Starting transcoding job {}: {}", job.id, job.source_path.display());
                info!("⚠️  Only one job runs at a time - GPU will be dedicated to this job");

                job.status = JobStatus::Running;
                job.started_at = Some(Utc::now());
                save_job(job, &cfg.job_state_dir)?;

                // Process the job (this will block until complete)
                match process_job(&cfg, &ffmpeg_mgr, job).await {
                    Ok(()) => {
                        info!("✅ Job {} completed successfully", job.id);
                    }
                    Err(e) => {
                        error!("❌ Job {} failed: {}", job.id, e);
                        job.status = JobStatus::Failed;
                        job.reason = Some(format!("{}", e));
                        job.finished_at = Some(Utc::now());
                        save_job(job, &cfg.job_state_dir)?;
                    }
                }
            } else if pending_count == 0 {
                debug!("No pending jobs, waiting for next scan");
            }
        } else {
            // Job is already running - wait for it to complete
            let running_jobs: Vec<_> = jobs.iter()
                .filter(|j| j.status == JobStatus::Running)
                .map(|j| j.id.clone())
                .collect();
            info!("⏸️  Waiting for {} running job(s) to complete before starting next: {:?}", 
                  running_count, running_jobs);
            if pending_count > 0 {
                info!("   {} pending job(s) will start after current job(s) finish", pending_count);
            }
        }

        // Periodic cleanup of orphaned temp files (every 10 scans = ~10 minutes)
        if scan_count % 10 == 0 {
            info!("🧹 Running periodic temp file cleanup...");
            match cleanup_orphaned_temp_files(&cfg).await {
                Ok(count) => {
                    if count > 0 {
                        info!("✅ Periodic cleanup: removed {} orphaned temp file(s)", count);
                    } else {
                        debug!("Periodic cleanup: no orphaned files found");
                    }
                }
                Err(e) => {
                    warn!("⚠️  Periodic cleanup failed (non-fatal): {}", e);
                }
            }
        }

        // Sleep before next scan
        info!("Sleeping for {} seconds before next scan", cfg.scan_interval_secs);
        tokio::time::sleep(tokio::time::Duration::from_secs(cfg.scan_interval_secs)).await;
    }
}

/// Track progress state for stuck job detection
#[derive(Debug, Clone)]
struct JobProgressState {
    last_temp_file_size: u64,
    last_check_time: DateTime<Utc>,
    last_file_mtime: Option<u64>, // Unix timestamp
}

// Use a function that maintains state via thread-local or pass it as parameter
// For simplicity, we'll pass state as a mutable reference through the call chain
fn check_file_activity_with_state(
    cfg: &TranscodeConfig,
    job: &Job,
    progress_state: &mut HashMap<String, JobProgressState>
) -> Result<(bool, u64, Option<u64>)> {
    let temp_output = get_temp_output_path(cfg, &job.source_path);
    
    if !temp_output.exists() {
        return Ok((false, 0, None));
    }
    
    let metadata = fs::metadata(&temp_output)
        .with_context(|| format!("Failed to stat temp file: {}", temp_output.display()))?;
    
    let current_size = metadata.len();
    let current_mtime = metadata.modified()
        .ok()
        .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs());
    
    let has_activity = if let Some(state) = progress_state.get(&job.id) {
        // Check if file has grown or mtime has changed
        let size_changed = current_size > state.last_temp_file_size;
        let mtime_changed = current_mtime != state.last_file_mtime;
        
        // Update state
        progress_state.insert(job.id.clone(), JobProgressState {
            last_temp_file_size: current_size,
            last_check_time: Utc::now(),
            last_file_mtime: current_mtime,
        });
        
        size_changed || mtime_changed
    } else {
        // First check - initialize state
        progress_state.insert(job.id.clone(), JobProgressState {
            last_temp_file_size: current_size,
            last_check_time: Utc::now(),
            last_file_mtime: current_mtime,
        });
        true // Assume activity on first check
    };
    
    Ok((has_activity, current_size, current_mtime))
}




/// Recover stuck jobs - check for jobs in Running status that are actually abandoned
/// Uses advanced multi-signal detection: process existence, file activity, time-based
/// Returns the number of jobs recovered
async fn recover_stuck_jobs(cfg: &TranscodeConfig) -> Result<usize> {
    info!("🔍 Checking for stuck jobs (advanced multi-signal detection)...");
    
    let jobs = load_all_jobs(&cfg.job_state_dir)
        .context("Failed to load jobs for recovery")?;
    
    let stuck_timeout = chrono::Duration::seconds(cfg.stuck_job_timeout_secs as i64);
    let file_inactivity_timeout = chrono::Duration::seconds(cfg.stuck_job_file_inactivity_secs as i64);
    let mut recovered_count = 0;
    let now = Utc::now();
    
    // Track progress state for file activity checks
    let mut progress_state = HashMap::<String, JobProgressState>::new();
    
    for mut job in jobs {
        if job.status != JobStatus::Running {
            continue;
        }
        
        let mut stuck_reasons = Vec::new();
        let mut is_stuck = false;
        
        // Signal 1: Time-based check
        let age = if let Some(started) = job.started_at {
            now - started
        } else {
            // Job has Running status but no started_at - definitely stuck
            stuck_reasons.push("no started_at timestamp".to_string());
            recover_job_safely(cfg, &mut job, now, &stuck_reasons.join(", "), &mut progress_state)?;
            recovered_count += 1;
            continue;
        };
        
        if age > stuck_timeout {
            is_stuck = true;
            stuck_reasons.push(format!("time-based timeout (running for {})", format_duration(age)));
        }
        
        // Signal 2: Process check disabled for native FFmpeg (no Docker containers to check)
        // Native FFmpeg runs as direct subprocess, so we rely on file activity checks instead
        
        // Signal 3: File activity check (if enabled)
        if cfg.stuck_job_check_enable_file_activity && !is_stuck {
            match check_file_activity_with_state(cfg, &job, &mut progress_state) {
                Ok((has_activity, _current_size, current_mtime)) => {
                    if let Some(state) = progress_state.get(&job.id) {
                        let time_since_last_activity = now - state.last_check_time;
                        
                        // Check if file hasn't grown/changed in inactivity timeout
                        if !has_activity && time_since_last_activity > file_inactivity_timeout {
                            // Additional check: if temp file exists but mtime is old
                            if let Some(mtime_secs) = current_mtime {
                                if let Some(mtime_dt) = DateTime::from_timestamp(mtime_secs as i64, 0) {
                                    let mtime_age = now - mtime_dt;
                                    
                                    if mtime_age > file_inactivity_timeout {
                                        is_stuck = true;
                                        stuck_reasons.push(format!("temp file inactive for {} (no growth/mtime change)", format_duration(time_since_last_activity)));
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    debug!("Failed to check file activity for job {}: {}", job.id, e);
                    // If temp file doesn't exist and job has been running for a while, might be stuck
                    if age > chrono::Duration::minutes(5) {
                        is_stuck = true;
                        stuck_reasons.push("temp file missing (job running > 5 minutes)".to_string());
                    }
                }
            }
        }
        
        // If job is stuck, recover it
        if is_stuck {
            warn!("Job {}: ⚠️  Found stuck job - {}", job.id, stuck_reasons.join("; "));
            recover_job_safely(cfg, &mut job, now, &stuck_reasons.join(", "), &mut progress_state)?;
            recovered_count += 1;
        }
    }
    
    if recovered_count > 0 {
        info!("✅ Recovered {} stuck job(s)", recovered_count);
    } else {
        debug!("No stuck jobs found");
    }
    
    Ok(recovered_count)
}

/// Helper function to safely recover a stuck job (cleanup and reset)
fn recover_job_safely(
    cfg: &TranscodeConfig, 
    job: &mut Job, 
    now: DateTime<Utc>, 
    reason: &str,
    progress_state: &mut HashMap<String, JobProgressState>
) -> Result<()> {
    let temp_output = get_temp_output_path(cfg, &job.source_path);
    let orig_backup = job.source_path.with_extension("orig.mkv");
    
    // Clean up temp file if it exists (abandoned transcode)
    if temp_output.exists() {
        fs::remove_file(&temp_output)
            .with_context(|| format!("Failed to delete temp file: {}", temp_output.display()))?;
        info!("Job {}: 🗑️  Deleted abandoned temp file: {}", job.id, temp_output.display());
    }
    
    // Check file states and recover appropriately
    let orig_exists = job.source_path.exists();
    let backup_exists = orig_backup.exists();
    
    if !orig_exists && backup_exists {
        // Failed replacement - restore backup
        fs::rename(&orig_backup, &job.source_path)
            .with_context(|| format!("Failed to restore backup: {} -> {}", 
                orig_backup.display(), job.source_path.display()))?;
        info!("Job {}: 🔄 Restored original from backup: {}", job.id, job.source_path.display());
        job.status = JobStatus::Pending;
        job.started_at = None;
    } else if !orig_exists && !backup_exists {
        // Original missing, no backup - corrupted state
        error!("Job {}: ❌ Original file missing with no backup - marking as Failed", job.id);
        job.status = JobStatus::Failed;
        job.reason = Some(format!("recovery: {} (original file missing, no backup)", reason));
        job.finished_at = Some(now);
    } else if orig_exists {
        // Original exists - can restart transcode
        job.status = JobStatus::Pending;
        job.started_at = None;
    }
    
    // Clear reason if resetting to Pending
    if job.status == JobStatus::Pending {
        job.reason = None;
    }
    
    save_job(job, &cfg.job_state_dir)?;
    info!("Job {}: 🔄 Recovered stuck job - reset to {:?}", job.id, job.status);
    
    // Clean up progress state
    progress_state.remove(&job.id);
    
    Ok(())
}

/// Helper function to format duration for logging
fn format_duration(d: chrono::Duration) -> String {
    let hours = d.num_hours();
    let minutes = d.num_minutes() % 60;
    let seconds = d.num_seconds() % 60;
    format!("{}h {}m {}s", hours, minutes, seconds)
}

/// Force requeue a job - stop container, clean files, reset to Pending
/// Performs safe cleanup with proper error handling
async fn force_requeue_job(cfg: &TranscodeConfig, job: &mut Job) -> Result<()> {
    
    info!("🔄 Force requeue requested for job {}: {}", job.id, job.source_path.display());
    
    let temp_output = get_temp_output_path(cfg, &job.source_path);
    let orig_backup = job.source_path.with_extension("orig.mkv");
    
    // Step 1: No Docker containers to stop (native FFmpeg runs as direct subprocess)
    // If a job is stuck, the subprocess should have already terminated or will be orphaned
    info!("Job {}: Native FFmpeg mode - no containers to stop", job.id);
    
    // Step 2: Clean up temp file
    if temp_output.exists() {
        fs::remove_file(&temp_output)
            .with_context(|| format!("Failed to delete temp file: {}", temp_output.display()))?;
        info!("Job {}: 🗑️  Deleted temp file: {}", job.id, temp_output.display());
    }
    
    // Step 3: Restore original file if needed
    if !job.source_path.exists() && orig_backup.exists() {
        fs::rename(&orig_backup, &job.source_path)
            .with_context(|| format!("Failed to restore backup: {} -> {}", 
                orig_backup.display(), job.source_path.display()))?;
        info!("Job {}: 🔄 Restored original from backup: {}", job.id, job.source_path.display());
    } else if orig_backup.exists() && job.source_path.exists() {
        // Both exist - delete backup since original is intact
        fs::remove_file(&orig_backup)
            .with_context(|| format!("Failed to delete backup file: {}", orig_backup.display()))?;
        info!("Job {}: 🗑️  Deleted backup file (original exists): {}", job.id, orig_backup.display());
    }
    
    // Step 4: Reset job state
    job.status = JobStatus::Pending;
    job.started_at = None;
    job.finished_at = None;
    job.reason = None;
    job.output_path = None;
    job.new_bytes = None;
    
    // Step 5: Save job state
    save_job(job, &cfg.job_state_dir)?;
    info!("Job {}: ✅ Force requeue complete - job reset to Pending", job.id);
    
    Ok(())
}

/// Command file format for TUI communication
#[derive(serde::Serialize, serde::Deserialize, Debug)]
struct CommandFile {
    action: String,
    job_id: String,
    reason: Option<String>,
    timestamp: String,
}

/// Process command files from TUI
async fn process_command_files(cfg: &TranscodeConfig) -> Result<usize> {
    let command_dir = cfg.command_dir();
    
    // Create command directory if it doesn't exist
    if !command_dir.exists() {
        fs::create_dir_all(&command_dir)
            .with_context(|| format!("Failed to create command directory: {}", command_dir.display()))?;
    }
    
    let mut processed_count = 0;
    
    // Read all command files
    let entries = match fs::read_dir(&command_dir) {
        Ok(entries) => entries,
        Err(e) => {
            debug!("Failed to read command directory {}: {}", command_dir.display(), e);
            return Ok(0);
        }
    };
    
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        
        // Read and parse command file
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                warn!("Failed to read command file {}: {}", path.display(), e);
                continue;
            }
        };
        
        let cmd: CommandFile = match serde_json::from_str(&content) {
            Ok(c) => c,
            Err(e) => {
                warn!("Failed to parse command file {}: {}", path.display(), e);
                // Delete invalid command file
                fs::remove_file(&path).ok();
                continue;
            }
        };
        
        // Process command
        if cmd.action == "requeue" {
            // Load job
            let jobs = load_all_jobs(&cfg.job_state_dir)
                .context("Failed to load jobs for requeue")?;
            
            if let Some(mut job) = jobs.into_iter().find(|j| j.id == cmd.job_id) {
                // Verify job is in a state that can be requeued
                if job.status == JobStatus::Running || job.status == JobStatus::Pending {
                    info!("Job {}: Processing manual requeue command", job.id);
                    
                    if job.status == JobStatus::Running {
                        // Force requeue with cleanup
                        force_requeue_job(cfg, &mut job).await
                            .with_context(|| format!("Failed to force requeue job {}", job.id))?;
                    } else {
                        // Already pending, just log it
                        info!("Job {}: Already pending, no action needed", job.id);
                    }
                    
                    processed_count += 1;
                } else {
                    warn!("Job {}: Cannot requeue - status is {:?}", job.id, job.status);
                }
            } else {
                warn!("Job {}: Not found for requeue command", cmd.job_id);
            }
        } else {
            warn!("Unknown command action: {}", cmd.action);
        }
        
        // Delete command file after processing
        fs::remove_file(&path)
            .with_context(|| format!("Failed to delete processed command file: {}", path.display()))?;
    }
    
    if processed_count > 0 {
        info!("✅ Processed {} command file(s)", processed_count);
    }
    
    Ok(processed_count)
}

/// Clean up orphaned temp files that don't have active jobs
/// Returns the number of files cleaned up
/// Aggressively cleans both library roots AND temp_output_dir
async fn cleanup_orphaned_temp_files(cfg: &TranscodeConfig) -> Result<usize> {
    info!("🔍 Checking for orphaned temp files in library and temp directory...");
    
    // Load all jobs to check against
    let jobs = load_all_jobs(&cfg.job_state_dir)
        .context("Failed to load jobs for cleanup")?;
    
    // Create set of filenames that have active jobs (just the stem, not full path)
    let active_filenames: HashSet<String> = jobs
        .iter()
        .filter(|j| matches!(j.status, JobStatus::Pending | JobStatus::Running))
        .filter_map(|j| {
            j.source_path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
        })
        .collect();
    
    let mut cleaned_count = 0;
    
    // PRIORITY 1: Clean temp_output_dir (NVMe) - this is where space matters most!
    info!("🧹 Cleaning temp directory: {}", cfg.temp_output_dir.display());
    if cfg.temp_output_dir.exists() {
        let temp_dir = cfg.temp_output_dir.clone();
        let active_names = active_filenames.clone();
        
        let temp_cleaned = tokio::task::spawn_blocking(move || {
            let mut count = 0;
            if let Ok(entries) = fs::read_dir(&temp_dir) {
                for entry in entries.filter_map(|e| e.ok()) {
                    let path = entry.path();
                    if !path.is_file() {
                        continue;
                    }
                    
                    // Check if it's a temp file
                    if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                        if !file_name.ends_with(".tmp.av1.mkv") {
                            continue;
                        }
                        
                        // Extract base filename (remove .tmp.av1.mkv)
                        let base_name = file_name.trim_end_matches(".tmp.av1.mkv");
                        
                        // Check if there's an active job for this file
                        if !active_names.contains(base_name) {
                            // No active job - delete it!
                            if let Ok(metadata) = fs::metadata(&path) {
                                let size_mb = metadata.len() as f64 / 1_000_000.0;
                                info!("🗑️  Deleting orphaned temp file: {} ({:.1} MB)", file_name, size_mb);
                            }
                            
                            if fs::remove_file(&path).is_ok() {
                                count += 1;
                            }
                        }
                    }
                }
            }
            count
        }).await.context("Failed to clean temp directory")?;
        
        cleaned_count += temp_cleaned;
        if temp_cleaned > 0 {
            info!("✅ Cleaned {} orphaned file(s) from temp directory", temp_cleaned);
        }
    } else {
        warn!("⚠️  Temp directory does not exist: {}", cfg.temp_output_dir.display());
    }
    
    // Note: We don't scan library roots anymore since temp files are now exclusively in temp_output_dir
    // This is much faster and cleaner!
    
    if cleaned_count == 0 {
        debug!("No orphaned temp files found");
    }
    
    Ok(cleaned_count)
}

/// Extract metadata for a pending job (background task for EST SAVE calculation)
async fn extract_metadata_for_job(
    cfg: &TranscodeConfig,
    job_id: &str,
    job_path: &PathBuf,
    job_state_dir: &PathBuf,
) -> Result<()> {
    debug!("Job {}: Starting background metadata extraction for {}", job_id, job_path.display());
    
    // Run ffprobe to get metadata
    let meta = match ffprobe::probe_file(cfg, job_path).await {
        Ok(m) => m,
        Err(e) => {
            warn!("Job {}: Background ffprobe failed: {}", job_id, e);
            return Err(e).with_context(|| format!("Failed to probe file: {}", job_path.display()));
        }
    };
    
    debug!("Job {}: Background ffprobe completed, found {} streams", job_id, meta.streams.len());
    
    // Load the job from disk (might have been updated by another process)
    let all_jobs = load_all_jobs(job_state_dir)
        .context("Failed to load jobs for metadata extraction")?;
    let mut job = match all_jobs.iter().find(|j| j.id == job_id) {
        Some(j) => j.clone(),
        None => {
            warn!("Job {}: Not found in job list (might have been deleted)", job_id);
            return Ok(()); // Job doesn't exist, nothing to do
        }
    };
    
    // Only update if job is still pending and metadata is missing
    if job.status != JobStatus::Pending {
        debug!("Job {}: Not pending anymore, skipping metadata extraction", job_id);
        return Ok(());
    }
    
    // Check if metadata is already complete (avoid redundant extraction)
    if job.video_codec.is_some() && 
       job.video_width.is_some() && 
       job.video_height.is_some() && 
       job.video_bitrate.is_some() && 
       job.video_frame_rate.is_some() {
        debug!("Job {}: Already has complete metadata, skipping extraction", job_id);
        return Ok(());
    }
    
    // Check for video streams
    let video_streams: Vec<_> = meta.streams
        .iter()
        .filter(|s| s.codec_type.as_deref() == Some("video"))
        .collect();
    
    if video_streams.is_empty() {
        debug!("Job {}: No video streams found, skipping metadata extraction", job_id);
        return Ok(());
    }
    
    // Extract and store video metadata
    if let Some(video_stream) = video_streams.first() {
        job.video_codec = video_stream.codec_name.clone();
        job.video_width = video_stream.width;
        job.video_height = video_stream.height;
        job.video_frame_rate = video_stream.avg_frame_rate.clone();
        
        // Get bitrate from video stream first, fallback to format bitrate
        if let Some(stream_bitrate_str) = &video_stream.bit_rate {
            if let Ok(bitrate) = stream_bitrate_str.parse::<u64>() {
                job.video_bitrate = Some(bitrate);
            }
        }
        if job.video_bitrate.is_none() {
            if let Some(format_bitrate_str) = &meta.format.bit_rate {
                if let Ok(bitrate) = format_bitrate_str.parse::<u64>() {
                    job.video_bitrate = Some(bitrate);
                }
            }
        }
        
        // Verify all required metadata is present before saving
        let has_all_metadata = job.video_codec.is_some() && 
                               job.video_width.is_some() && 
                               job.video_height.is_some() && 
                               job.video_bitrate.is_some() && 
                               job.video_frame_rate.is_some();
        
        if !has_all_metadata {
            warn!("Job {}: Background metadata extraction incomplete - missing fields: codec={:?}, width={:?}, height={:?}, bitrate={:?}, fps={:?}", 
                  job_id, job.video_codec, job.video_width, job.video_height, job.video_bitrate, job.video_frame_rate);
        }
        
        // Save job with metadata
        save_job(&job, job_state_dir)
            .context("Failed to save job with metadata")?;
        
        // Verify the save worked by reloading and checking
        if let Ok(verify_jobs) = load_all_jobs(job_state_dir) {
            if let Some(saved_job) = verify_jobs.iter().find(|j| j.id == job_id) {
                let saved_has_metadata = saved_job.video_codec.is_some() && 
                                        saved_job.video_width.is_some() && 
                                        saved_job.video_height.is_some() && 
                                        saved_job.video_bitrate.is_some() && 
                                        saved_job.video_frame_rate.is_some();
                if saved_has_metadata {
                    info!("Job {}: ✅ Background metadata extraction complete - EST SAVE now available in TUI (verified: codec={:?}, {:.0}x{:.0}, bitrate={:?} bps, fps={:?})", 
                          job_id, 
                          saved_job.video_codec, 
                          saved_job.video_width.unwrap_or(0), 
                          saved_job.video_height.unwrap_or(0),
                          saved_job.video_bitrate,
                          saved_job.video_frame_rate);
                } else {
                    warn!("Job {}: ⚠️  Background metadata extraction saved but verification failed - metadata may be incomplete", job_id);
                }
            } else {
                warn!("Job {}: ⚠️  Background metadata extraction saved but job not found on reload", job_id);
            }
        } else {
            warn!("Job {}: ⚠️  Background metadata extraction saved but failed to verify (could not reload jobs)", job_id);
        }
    }
    
    Ok(())
}

/// Process a single job: probe, classify, transcode, and apply size gate
async fn process_job(cfg: &TranscodeConfig, ffmpeg_mgr: &FFmpegManager, job: &mut Job) -> Result<()> {
    info!("Job {}: Starting ffprobe for {}", job.id, job.source_path.display());
    
    // Step 1: Run ffprobe to get metadata
    let meta = match ffprobe::probe_file_native(ffmpeg_mgr, &job.source_path).await {
        Ok(m) => m,
        Err(e) => {
            error!("Job {}: ffprobe failed: {}", job.id, e);
            return Err(e).with_context(|| format!("Failed to probe file: {}", job.source_path.display()));
        }
    };
    
    info!("Job {}: ffprobe completed, found {} streams", job.id, meta.streams.len());

    // Step 2: Check for video streams
    let video_streams: Vec<_> = meta.streams
        .iter()
        .filter(|s| s.codec_type.as_deref() == Some("video"))
        .collect();

    info!("Job {}: Found {} video streams", job.id, video_streams.len());

    if video_streams.is_empty() {
        let reason = "not a video".to_string();
        info!("Job {}: Skipping - {}", job.id, reason);
        sidecar::write_why_txt(&job.source_path, &reason)?;
        job.status = JobStatus::Skipped;
        job.reason = Some(reason);
        job.finished_at = Some(Utc::now());
        save_job(job, &cfg.job_state_dir)?;
        return Ok(());
    }

    // Step 3: Check if already AV1
    let video_codecs: Vec<&str> = video_streams.iter()
        .filter_map(|s| s.codec_name.as_deref())
        .collect();
    info!("Job {}: Video codecs found: {:?}", job.id, video_codecs);
    
    if video_streams.iter().any(|s| s.codec_name.as_deref() == Some("av1")) {
        let reason = "already av1".to_string();
        info!("Job {}: Skipping - {}", job.id, reason);
        sidecar::write_why_txt(&job.source_path, &reason)?;
        job.status = JobStatus::Skipped;
        job.reason = Some(reason);
        job.finished_at = Some(Utc::now());
        save_job(job, &cfg.job_state_dir)?;
        return Ok(());
    }

    info!("Job {}: File is not AV1, proceeding with transcode", job.id);

    // Extract and store video metadata for estimation
    info!("Job {}: Extracting video metadata for estimation...", job.id);
    if let Some(video_stream) = video_streams.first() {
        info!("Job {}: Found video stream, extracting metadata...", job.id);
        job.video_codec = video_stream.codec_name.clone();
        job.video_width = video_stream.width;
        job.video_height = video_stream.height;
        job.video_frame_rate = video_stream.avg_frame_rate.clone();
        
        // Extract bit depth and HDR information
        let bit_depth = video_stream.detect_bit_depth();
        let is_hdr = video_stream.is_hdr_content();
        
        job.source_bit_depth = Some(match bit_depth {
            daemon::BitDepth::Bit8 => 8,
            daemon::BitDepth::Bit10 => 10,
            daemon::BitDepth::Unknown => 8,
        });
        job.is_hdr = Some(is_hdr);
        job.source_pix_fmt = video_stream.pix_fmt.clone();
        
        info!("Job {}: Extracted basic metadata - codec: {:?}, width: {:?}, height: {:?}, fps: {:?}, bit_depth: {}-bit{}, pix_fmt: {:?}", 
              job.id, job.video_codec, job.video_width, job.video_height, job.video_frame_rate,
              job.source_bit_depth.unwrap_or(8),
              if is_hdr { " HDR" } else { "" },
              job.source_pix_fmt);
        
        // Get bitrate from video stream first, fallback to format bitrate
        if let Some(stream_bitrate_str) = &video_stream.bit_rate {
            info!("Job {}: Trying to parse stream bitrate: {}", job.id, stream_bitrate_str);
            if let Ok(bitrate) = stream_bitrate_str.parse::<u64>() {
                job.video_bitrate = Some(bitrate);
                info!("Job {}: Parsed stream bitrate: {} bps", job.id, bitrate);
            } else {
                warn!("Job {}: Failed to parse stream bitrate: {}", job.id, stream_bitrate_str);
            }
        } else {
            info!("Job {}: No stream bitrate found, trying format bitrate...", job.id);
        }
        if job.video_bitrate.is_none() {
            if let Some(format_bitrate_str) = &meta.format.bit_rate {
                info!("Job {}: Trying to parse format bitrate: {}", job.id, format_bitrate_str);
                if let Ok(bitrate) = format_bitrate_str.parse::<u64>() {
                    job.video_bitrate = Some(bitrate);
                    info!("Job {}: Parsed format bitrate: {} bps", job.id, bitrate);
                } else {
                    warn!("Job {}: Failed to parse format bitrate: {}", job.id, format_bitrate_str);
                }
            } else {
                warn!("Job {}: No format bitrate found either", job.id);
            }
        }
        
        // Validate all required metadata is present for estimation
        let has_codec = job.video_codec.is_some();
        let has_width = job.video_width.is_some();
        let has_height = job.video_height.is_some();
        let has_bitrate = job.video_bitrate.is_some();
        let has_fps = job.video_frame_rate.is_some();
        
        info!("Job {}: Metadata validation - codec: {}, width: {}, height: {}, bitrate: {}, fps: {}", 
              job.id, has_codec, has_width, has_height, has_bitrate, has_fps);
        
        if has_codec && has_width && has_height && has_bitrate && has_fps {
            info!("Job {}: ✅ Video metadata COMPLETE - codec: {:?}, resolution: {:?}x{:?}, bitrate: {:?} bps, fps: {:?} (ESTIMATION WILL WORK)", 
                  job.id, job.video_codec, job.video_width, job.video_height, job.video_bitrate, job.video_frame_rate);
        } else {
            warn!("Job {}: ❌ Video metadata INCOMPLETE - codec: {}, width: {}, height: {}, bitrate: {}, fps: {} (ESTIMATION WILL NOT WORK)", 
                  job.id, has_codec, has_width, has_height, has_bitrate, has_fps);
            warn!("Job {}: Missing fields - codec: {:?}, width: {:?}, height: {:?}, bitrate: {:?}, fps: {:?}", 
                  job.id, job.video_codec, job.video_width, job.video_height, job.video_bitrate, job.video_frame_rate);
        }
        
        // Save job immediately after extracting metadata so TUI can use it
        info!("Job {}: Saving job with metadata...", job.id);
        match save_job(job, &cfg.job_state_dir) {
            Ok(()) => {
                info!("Job {}: ✅ Job saved successfully with metadata", job.id);
                // Verify the save worked by checking if we can read it back
                if let Ok(verify_job) = job::load_all_jobs(&cfg.job_state_dir) {
                    if let Some(saved_job) = verify_job.iter().find(|j| j.id == job.id) {
                        let has_meta = saved_job.video_codec.is_some() 
                            && saved_job.video_width.is_some() 
                            && saved_job.video_height.is_some()
                            && saved_job.video_bitrate.is_some()
                            && saved_job.video_frame_rate.is_some();
                        if has_meta {
                            info!("Job {}: ✅ Verified - metadata is saved in job file", job.id);
                        } else {
                            warn!("Job {}: ⚠️  WARNING - metadata not found in saved job file!", job.id);
                        }
                    }
                }
            },
            Err(e) => error!("Job {}: ❌ Failed to save job: {}", job.id, e),
        }
    } else {
        warn!("Job {}: ❌ No video stream found - cannot extract metadata for estimation", job.id);
    }

    // Step 4: Classify source using enhanced SourceClassifier
    let classifier = classifier::SourceClassifier::new();
    let classification = classifier.classify(&job.source_path, &meta.format, &meta.streams);
    
    // Store quality tier in job
    job.quality_tier = Some(match classification.tier {
        QualityTier::Remux => "Remux".to_string(),
        QualityTier::WebDl => "WebDl".to_string(),
        QualityTier::LowQuality => "LowQuality".to_string(),
    });
    
    info!("Job {}: 🎯 Source classification: {:?} (confidence: {:.2})", 
          job.id, classification.tier, classification.confidence);
    info!("Job {}: 📋 Classification reasons:", job.id);
    for reason in &classification.reasons {
        info!("Job {}:    - {}", job.id, reason);
    }
    
    // Check if we should skip re-encoding for clean WEB-DL sources
    if classifier.should_skip_encode(&classification, &meta.streams) {
        let reason = format!("clean {:?} source with modern codec - skipping re-encode", classification.tier);
        info!("Job {}: Skipping - {}", job.id, reason);
        sidecar::write_why_txt(&job.source_path, &reason)?;
        job.status = JobStatus::Skipped;
        job.reason = Some(reason);
        job.finished_at = Some(Utc::now());
        save_job(job, &cfg.job_state_dir)?;
        return Ok(());
    }
    
    // Log encoding strategy based on classification
    match classification.tier {
        QualityTier::Remux => {
            info!("Job {}: 💎 Using REMUX encoding strategy (quality-first, test clip workflow)", job.id);
        }
        QualityTier::WebDl => {
            info!("Job {}: 🌐 Using WEB-DL encoding strategy (conservative re-encoding)", job.id);
        }
        QualityTier::LowQuality => {
            info!("Job {}: 📦 Using LOW-QUALITY encoding strategy (size optimization)", job.id);
        }
    }

    // Step 5: Generate temp output path
    let temp_output = get_temp_output_path(cfg, &job.source_path);
    info!("Job {}: Using fast temp directory: {}", job.id, cfg.temp_output_dir.display());
    
    // Ensure temp directory exists
    fs::create_dir_all(&cfg.temp_output_dir)
        .with_context(|| format!("Failed to create temp output directory: {}", cfg.temp_output_dir.display()))?;
    
    info!("Job {}: Temp output will be: {}", job.id, temp_output.display());

    // Step 6: Calculate encoding parameters using QualityCalculator
    let quality_calc = QualityCalculator::new();
    let encoding_params = quality_calc.calculate_params(
        &classification,
        &meta,
        ffmpeg_mgr.best_encoder(),
    );
    
    // Store encoding parameters in job
    job.crf_used = Some(encoding_params.crf);
    job.preset_used = Some(encoding_params.preset);
    job.encoder_used = Some(format!("{:?}", ffmpeg_mgr.best_encoder()));
    job.target_bit_depth = Some(match encoding_params.bit_depth {
        daemon::BitDepth::Bit8 => 8,
        daemon::BitDepth::Bit10 => 10,
        daemon::BitDepth::Unknown => 8,
    });
    
    info!("Job {}: Encoding plan - Source: {}-bit{} → Target: {}-bit AV1",
          job.id,
          job.source_bit_depth.unwrap_or(8),
          if job.is_hdr.unwrap_or(false) { " HDR" } else { "" },
          job.target_bit_depth.unwrap_or(8)
    );
    info!("Job {}: Encoder: {:?}, CRF: {}, Preset: {}", 
          job.id, ffmpeg_mgr.best_encoder(), encoding_params.crf, encoding_params.preset);
    if let Some(tune) = encoding_params.tune {
        info!("Job {}: Tune: {}", job.id, tune);
    }
    if let Some(film_grain) = encoding_params.film_grain {
        info!("Job {}: Film grain: {}", job.id, film_grain);
    }
    
    // Save job with encoding parameters before encoding starts
    save_job(job, &cfg.job_state_dir)?;
    
    // Step 7: Test clip workflow for REMUX sources
    if matches!(classification.tier, QualityTier::Remux) && cfg.enable_test_clip_workflow {
        info!("Job {}: 🎬 Starting test clip workflow for REMUX source", job.id);
        
        let test_clip_workflow = TestClipWorkflow::new(cfg.temp_output_dir.clone());
        
        // Extract test clip
        let test_clip_info = match test_clip_workflow.extract_test_clip(&job.source_path, &meta, ffmpeg_mgr).await {
            Ok(info) => {
                info!("Job {}: ✅ Test clip extracted: {} ({:.1}s at {:.1}s)", 
                      job.id, info.clip_path.display(), info.duration, info.start_time);
                job.test_clip_path = Some(info.clip_path.clone());
                save_job(job, &cfg.job_state_dir)?;
                Some(info)
            }
            Err(e) => {
                warn!("Job {}: ⚠️  Test clip extraction failed (non-fatal): {}", job.id, e);
                warn!("Job {}: Proceeding with full encode without test clip", job.id);
                // Continue without test clip
                None
            }
        };
        
        // If test clip was extracted, encode it and await user approval
        if let Some(clip_info) = test_clip_info {
            info!("Job {}: 🎬 Encoding test clip with proposed parameters...", job.id);
            
            let test_output = match test_clip_workflow.encode_test_clip(
                &clip_info,
                &encoding_params,
                ffmpeg_mgr,
                &meta,
            ).await {
                Ok(output) => {
                    info!("Job {}: ✅ Test clip encoded: {}", job.id, output.display());
                    Some(output)
                }
                Err(e) => {
                    warn!("Job {}: ⚠️  Test clip encoding failed (non-fatal): {}", job.id, e);
                    warn!("Job {}: Proceeding with full encode", job.id);
                    // Clean up test clip
                    if clip_info.clip_path.exists() {
                        fs::remove_file(&clip_info.clip_path).ok();
                    }
                    // Continue without test clip approval
                    None
                }
            };
            
            // If test clip encoded successfully, auto-approve for now
            // TODO: Implement actual user approval mechanism (TUI command, file-based, etc.)
            if test_output.is_some() {
                info!("Job {}: ✅ Test clip encoded successfully - auto-approving for now", job.id);
                info!("Job {}: TODO: Implement user approval mechanism", job.id);
                job.test_clip_approved = Some(true);
                save_job(job, &cfg.job_state_dir)?;
            }
        }
    }
    
    // Step 8: Run full transcoding
    info!("Job {}: Starting ffmpeg transcoding with CRF: {}, Preset: {}...", 
          job.id, encoding_params.crf, encoding_params.preset);
    
    // Build FFmpeg command
    let cmd_builder = CommandBuilder::new();
    let ffmpeg_args = cmd_builder.build_encode_command(
        &job.source_path,
        &temp_output,
        &encoding_params,
        ffmpeg_mgr.best_encoder(),
        &meta,
    );
    
    info!("Job {}: FFmpeg command: ffmpeg {}", job.id, ffmpeg_args.join(" "));
    
    // Execute FFmpeg (no timeout - let it run as long as needed)
    let ffmpeg_result = match ffmpeg_mgr.execute_ffmpeg(ffmpeg_args, None).await {
        Ok(result) => result,
        Err(e) => {
            error!("Job {}: Failed to execute ffmpeg command: {}", job.id, e);
            let reason = format!("ffmpeg execution failed: {}", e);
            sidecar::write_why_txt(&job.source_path, &reason)?;
            // Delete temp file on failure
            if temp_output.exists() {
                fs::remove_file(&temp_output)
                    .with_context(|| format!("Failed to delete temp file after execution failure: {}", temp_output.display()))?;
                info!("Job {}: 🗑️  Deleted temp file after execution failure: {}", job.id, temp_output.display());
            }
            job.status = JobStatus::Failed;
            job.reason = Some(reason);
            job.finished_at = Some(Utc::now());
            save_job(job, &cfg.job_state_dir)?;
            return Ok(());
        }
    };
    
    if !ffmpeg_result.success {
        error!("Job {}: ffmpeg failed with exit code {:?}", job.id, ffmpeg_result.exit_code);
        error!("Job {}: ffmpeg STDOUT: {}", job.id, ffmpeg_result.stdout);
        error!("Job {}: ffmpeg STDERR: {}", job.id, ffmpeg_result.stderr);
        let reason = format!("ffmpeg exit code {:?}", ffmpeg_result.exit_code);
        sidecar::write_why_txt(&job.source_path, &reason)?;
        // Delete temp file on failure
        if temp_output.exists() {
            fs::remove_file(&temp_output)
                .with_context(|| format!("Failed to delete temp file after failure: {}", temp_output.display()))?;
            info!("Job {}: 🗑️  Deleted temp file after failure: {}", job.id, temp_output.display());
        }
        job.status = JobStatus::Failed;
        job.reason = Some(reason);
        job.finished_at = Some(Utc::now());
        save_job(job, &cfg.job_state_dir)?;
        return Ok(());
    }

    info!("Job {}: ffmpeg completed successfully (exit code 0)", job.id);

    // Step 7: Verify temp output file exists and is valid
    info!("Job {}: Verifying temp output file: {}", job.id, temp_output.display());
    
    // Check parent directory exists and is writable
    if let Some(parent) = temp_output.parent() {
        if !parent.exists() {
            error!("Job {}: Parent directory does not exist: {}", job.id, parent.display());
        } else {
            info!("Job {}: Parent directory exists: {}", job.id, parent.display());
            
            // List files in parent directory to see what's there
            if let Ok(entries) = fs::read_dir(parent) {
                let files: Vec<String> = entries
                    .filter_map(|e| e.ok())
                    .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
                    .filter(|name| name.contains("tmp.av1") || name.contains(&job.source_path.file_stem().and_then(|s| s.to_str()).unwrap_or("")))
                    .collect();
                info!("Job {}: Related files in directory: {:?}", job.id, files);
            }
        }
    }
    
    if !temp_output.exists() {
        let reason = format!("transcoded output file does not exist: {}", temp_output.display());
        error!("Job {}: {}", job.id, reason);
        error!("Job {}: FFmpeg reported success but output file is missing", job.id);
        error!("Job {}: This may indicate: disk full, permission issue, or path mismatch", job.id);
        
        sidecar::write_why_txt(&job.source_path, &reason)?;
        job.status = JobStatus::Failed;
        job.reason = Some(reason);
        job.finished_at = Some(Utc::now());
        save_job(job, &cfg.job_state_dir)?;
        return Ok(());
    }
    
    info!("Job {}: ✓ Temp output file exists", job.id);

    let new_metadata = fs::metadata(&temp_output)
        .with_context(|| format!("Failed to stat output file: {}", temp_output.display()))?;
    let new_bytes = new_metadata.len();

    // Verify temp file is not empty
    if new_bytes == 0 {
        let reason = "transcoded output file is empty".to_string();
        sidecar::write_why_txt(&job.source_path, &reason)?;
        fs::remove_file(&temp_output).ok(); // Clean up empty temp file
        job.status = JobStatus::Failed;
        job.reason = Some(reason);
        job.finished_at = Some(Utc::now());
        save_job(job, &cfg.job_state_dir)?;
        return Ok(());
    }

    // Step 9: Validate output file for corruption
    info!("Job {}: Running output validation to detect corruption...", job.id);
    
    // Use ffprobe to validate the output
    let _validation_result = match ffprobe::probe_file_native(ffmpeg_mgr, &temp_output).await {
        Ok(output_meta) => {
            // Check if output has video streams
            let has_video = output_meta.streams.iter().any(|s| s.codec_type.as_deref() == Some("video"));
            let has_av1 = output_meta.streams.iter().any(|s| s.codec_name.as_deref() == Some("av1"));
            
            if !has_video {
                let reason = "output validation failed: no video streams found".to_string();
                error!("Job {}: ❌ {}", job.id, reason);
                sidecar::write_why_txt(&job.source_path, &reason)?;
                fs::remove_file(&temp_output).ok();
                job.status = JobStatus::Failed;
                job.reason = Some(reason);
                job.finished_at = Some(Utc::now());
                save_job(job, &cfg.job_state_dir)?;
                return Ok(());
            }
            
            if !has_av1 {
                let reason = "output validation failed: output is not AV1".to_string();
                error!("Job {}: ❌ {}", job.id, reason);
                sidecar::write_why_txt(&job.source_path, &reason)?;
                fs::remove_file(&temp_output).ok();
                job.status = JobStatus::Failed;
                job.reason = Some(reason);
                job.finished_at = Some(Utc::now());
                save_job(job, &cfg.job_state_dir)?;
                return Ok(());
            }
            
            info!("Job {}: ✅ Output validation passed - AV1 video stream confirmed", job.id);
            true
        }
        Err(e) => {
            warn!("Job {}: ⚠️  Output validation failed to run (non-fatal): {}", job.id, e);
            // Don't fail the job if validation itself fails - just log it
            false
        }
    };

    // Step 8: Size gate check
    let orig_bytes = job.original_bytes.unwrap_or(0);
    if orig_bytes == 0 {
        // Fallback: get original size from file if not set
        let orig_meta = fs::metadata(&job.source_path)
            .with_context(|| format!("Failed to stat original file: {}", job.source_path.display()))?;
        job.original_bytes = Some(orig_meta.len());
    }
    let orig_bytes = job.original_bytes.unwrap_or(0);

    if new_bytes as f64 > orig_bytes as f64 * cfg.max_size_ratio {
        // Rejected by size gate
        let reason = format!(
            "rejected: new {} GB vs orig {} GB (>{}%)",
            new_bytes as f64 / 1_000_000_000.0,
            orig_bytes as f64 / 1_000_000_000.0,
            cfg.max_size_ratio * 100.0
        );
        sidecar::write_why_txt(&job.source_path, &reason)?;
        sidecar::write_skip_marker(&job.source_path)?;
        fs::remove_file(&temp_output).ok(); // Clean up temp file
        job.status = JobStatus::Skipped;
        job.reason = Some("size gate".to_string());
        job.finished_at = Some(Utc::now());
        save_job(job, &cfg.job_state_dir)?;
        return Ok(());
    }

    // Step 9: ALL VERIFICATIONS PASSED - Replace original file with transcoded version
    // This is the final step that ALWAYS executes if we reach here
    // Strategy: Backup original first, then replace with temp file
    
    // Verify original file still exists before backing up
    if !job.source_path.exists() {
        let reason = format!("original file no longer exists: {}", job.source_path.display());
        sidecar::write_why_txt(&job.source_path, &reason)?;
        fs::remove_file(&temp_output).ok(); // Clean up temp file
        job.status = JobStatus::Failed;
        job.reason = Some(reason);
        job.finished_at = Some(Utc::now());
        save_job(job, &cfg.job_state_dir)?;
        return Ok(());
    }

    let orig_backup = job.source_path.with_extension("orig.mkv");
    
    // Backup original file
    fs::rename(&job.source_path, &orig_backup)
        .with_context(|| format!("Failed to backup original file: {} -> {}", 
            job.source_path.display(), orig_backup.display()))?;

    // Replace with transcoded file
    // If temp file is on different filesystem (e.g., NVMe), rename will fail - use copy instead
    if let Err(e) = fs::rename(&temp_output, &job.source_path) {
        info!("Job {}: Rename failed ({}), copying from temp directory instead...", job.id, e);
        
        // Copy temp file to final location
        fs::copy(&temp_output, &job.source_path)
            .with_context(|| format!("Failed to copy transcoded file: {} -> {}", 
                temp_output.display(), job.source_path.display()))?;
        
        info!("Job {}: ✓ Copied transcoded file from temp directory", job.id);
        
        // SAFETY: Triple-check the destination file exists and is valid before deleting temp file
        // This prevents data loss if the copy was interrupted or incomplete
        let destination_valid = job.source_path.exists() 
            && fs::metadata(&job.source_path)
                .map(|m| m.len() > 1_000_000) // At least 1MB (sanity check)
                .unwrap_or(false);
        
        if !destination_valid {
            error!("Job {}: ❌ CRITICAL: Destination file missing or too small after copy! Keeping temp file as backup: {}", 
                   job.id, temp_output.display());
            // DO NOT delete temp file - it's our only copy!
        } else {
            info!("Job {}: ✅ Verified destination file is valid ({} bytes)", 
                  job.id, fs::metadata(&job.source_path).unwrap().len());
            
            // Now safe to delete temp file
            if temp_output.exists() {
                match fs::remove_file(&temp_output) {
                    Ok(_) => {
                        info!("Job {}: 🗑️  Deleted temp file from fast storage", job.id);
                    }
                    Err(e) => {
                        warn!("Job {}: ⚠️  Failed to delete temp file (non-fatal): {} - {}", 
                              job.id, temp_output.display(), e);
                    }
                }
            } else {
                info!("Job {}: ℹ️  Temp file already removed (possibly by another process)", job.id);
            }
        }
    } else {
        info!("Job {}: ✓ Moved transcoded file (same filesystem)", job.id);
    }

    // Verify replacement succeeded
    if !job.source_path.exists() {
        // Critical error: replacement failed, try to restore backup
        let _ = fs::rename(&orig_backup, &job.source_path); // Try to restore
        let reason = "file replacement verification failed - backup restored".to_string();
        sidecar::write_why_txt(&job.source_path, &reason)?;
        // Clean up temp file if it still exists
        if temp_output.exists() {
            fs::remove_file(&temp_output).ok();
        }
        job.status = JobStatus::Failed;
        job.reason = Some(reason);
        job.finished_at = Some(Utc::now());
        save_job(job, &cfg.job_state_dir)?;
        return Ok(());
    }

    // Step 10: ALL VERIFICATIONS PASSED - Delete original backup file
    // The transcoded file has successfully replaced the original
    // Now delete the .orig.mkv backup since everything worked
    if orig_backup.exists() {
        fs::remove_file(&orig_backup)
            .with_context(|| format!("Failed to delete original backup file: {}", orig_backup.display()))?;
        info!("Job {}: 🗑️  Deleted original backup file: {}", job.id, orig_backup.display());
    } else {
        warn!("Job {}: ⚠️  Original backup file not found (may have been deleted already): {}", job.id, orig_backup.display());
    }

    // Step 11: Update job status to Success - ALL CHECKS PASSED, FILE REPLACED, ORIGINAL DELETED
    let end_time = Utc::now();
    job.status = JobStatus::Success;
    job.output_path = Some(job.source_path.clone());
    job.new_bytes = Some(new_bytes);
    job.finished_at = Some(end_time);
    save_job(job, &cfg.job_state_dir)?;

    info!("Job {}: ✅ SUCCESS - Original file deleted, transcoded file in place (CRF: {}, Preset: {})", 
          job.id, job.crf_used.unwrap_or(0), job.preset_used.unwrap_or(0));
    
    // Step 12: Write simple completion marker
    info!("Job {}: 📝 Writing completion marker...", job.id);
    let completion_msg = format!(
        "Transcoded successfully\nEncoder: {:?}\nCRF: {}\nPreset: {}\nQuality Tier: {:?}\nOriginal: {} MB\nNew: {} MB\nSavings: {:.1}%",
        ffmpeg_mgr.best_encoder(),
        encoding_params.crf,
        encoding_params.preset,
        classification.tier,
        orig_bytes as f64 / 1_000_000.0,
        new_bytes as f64 / 1_000_000.0,
        (1.0 - (new_bytes as f64 / orig_bytes as f64)) * 100.0
    );
    
    match sidecar::write_why_txt(&job.source_path, &completion_msg) {
        Ok(()) => {
            info!("Job {}: ✅ Completion marker written", job.id);
        }
        Err(e) => {
            warn!("Job {}: ⚠️  Failed to write completion marker (non-fatal): {}", job.id, e);
        }
    }
    
    Ok(())
}

