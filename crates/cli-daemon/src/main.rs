use anyhow::{Context, Result};
use clap::Parser;
use daemon::{
    config::TranscodeConfig, 
    job::{self, Job, JobStatus, load_all_jobs, save_job},
    scan, ffprobe, classifier, ffmpeg_docker, sidecar,
};
use std::path::PathBuf;
use std::fs;
use std::collections::{HashMap, HashSet};
use chrono::{Utc, DateTime};
use log::{info, warn, error, debug};
use tokio::process::Command;

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
    loop {
        info!("Starting library scan...");

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
                match process_job(&cfg, job).await {
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
                
                // Extract metadata for pending jobs in background (for EST SAVE calculation in TUI)
                // Only extract metadata for jobs that don't have it yet
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
                    .take(1) // Only process one at a time to avoid overloading
                    .cloned()
                    .collect();
                
                if !pending_jobs_without_metadata.is_empty() {
                    let job = &pending_jobs_without_metadata[0];
                    info!("📊 Extracting metadata for pending job {} in background (for EST SAVE)...", job.id);
                    let cfg_clone = cfg.clone();
                    let job_id = job.id.clone();
                    let job_path = job.source_path.clone();
                    let job_state_dir = cfg.job_state_dir.clone();
                    
                    // Spawn background task to extract metadata
                    tokio::spawn(async move {
                        if let Err(e) = extract_metadata_for_job(&cfg_clone, &job_id, &job_path, &job_state_dir).await {
                            warn!("Failed to extract metadata for pending job {}: {}", job_id, e);
                        }
                    });
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
    job: &Job,
    progress_state: &mut HashMap<String, JobProgressState>
) -> Result<(bool, u64, Option<u64>)> {
    let temp_output = job.source_path.with_extension("tmp.av1.mkv");
    
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

/// Verify if Docker container is running for a job
/// Checks if any Docker containers exist with ffmpeg process matching the job's file path
async fn verify_docker_container_exists(cfg: &TranscodeConfig, job: &Job) -> Result<bool> {
    use std::process::Stdio;
    
    // Try to find Docker containers that might be running ffmpeg for this job
    // We'll check containers with volume mounts matching the job's parent directory
    
    let parent_dir = job.source_path.parent()
        .context("Job source path has no parent directory")?;
    
    // List running Docker containers
    let output = Command::new(&cfg.docker_bin)
        .arg("ps")
        .arg("--format")
        .arg("{{.ID}} {{.Command}} {{.Mounts}}")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("Failed to execute docker ps")?;
    
    if !output.status.success() {
        warn!("Docker ps failed, assuming container check unavailable");
        return Ok(true); // Assume container exists if we can't check (fail open)
    }
    
    let stdout = String::from_utf8_lossy(&output.stdout);
    
    // Check if any container has:
    // 1. ffmpeg in the command
    // 2. The parent directory in mounts
    // 3. The temp output file name pattern
    
    let temp_output = job.source_path.with_extension("tmp.av1.mkv");
    let output_basename = temp_output.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    
    for line in stdout.lines() {
        if line.contains("ffmpeg") || line.contains("ffprobe") {
            // Check if this container has the relevant volume mount
            if line.contains(parent_dir.to_str().unwrap_or("")) {
                // Found a container that might be ours
                // Additional check: see if it has the output file in the command
                if output_basename.is_empty() || line.contains(output_basename) {
                    debug!("Found potential Docker container for job {}: {}", job.id, line);
                    return Ok(true);
                }
            }
        }
    }
    
    debug!("No Docker container found for job {}", job.id);
    Ok(false)
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
            is_stuck = true;
            stuck_reasons.push("no started_at timestamp".to_string());
            recover_job_safely(cfg, &mut job, now, &stuck_reasons.join(", "), &mut progress_state)?;
            recovered_count += 1;
            continue;
        };
        
        if age > stuck_timeout {
            is_stuck = true;
            stuck_reasons.push(format!("time-based timeout (running for {})", format_duration(age)));
        }
        
        // Signal 2: Docker process check (if enabled)
        if cfg.stuck_job_check_enable_process && !is_stuck {
            match verify_docker_container_exists(cfg, &job).await {
                Ok(container_exists) => {
                    if !container_exists {
                        // Check how long container has been missing
                        if let Some(state) = progress_state.get(&job.id) {
                            let time_since_last_check = now - state.last_check_time;
                            // Container missing for more than 2 minutes
                            if time_since_last_check > chrono::Duration::minutes(2) {
                                is_stuck = true;
                                stuck_reasons.push(format!("Docker container missing for {}", format_duration(time_since_last_check)));
                            }
                        } else {
                            // First check, give it a grace period
                            // But if job has been running for > 2 minutes, container should exist
                            if age > chrono::Duration::minutes(2) {
                                is_stuck = true;
                                stuck_reasons.push("Docker container not found (job running > 2 minutes)".to_string());
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to check Docker container for job {}: {}", job.id, e);
                    // Fail open - don't mark as stuck if we can't check
                }
            }
        }
        
        // Signal 3: File activity check (if enabled)
        if cfg.stuck_job_check_enable_file_activity && !is_stuck {
            match check_file_activity_with_state(&job, &mut progress_state) {
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
    let temp_output = job.source_path.with_extension("tmp.av1.mkv");
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
    use std::process::Stdio;
    
    info!("🔄 Force requeue requested for job {}: {}", job.id, job.source_path.display());
    
    let temp_output = job.source_path.with_extension("tmp.av1.mkv");
    let orig_backup = job.source_path.with_extension("orig.mkv");
    
    // Step 1: Try to stop Docker container if it exists
    // Search for containers with ffmpeg that might be running this job
    let parent_dir = job.source_path.parent()
        .context("Job source path has no parent directory")?;
    
    // List all running Docker containers
    let output = Command::new(&cfg.docker_bin)
        .arg("ps")
        .arg("--format")
        .arg("{{.ID}} {{.Command}} {{.Mounts}}")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("Failed to execute docker ps")?;
    
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let output_basename = temp_output.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        
        // Find matching containers
        for line in stdout.lines() {
            if (line.contains("ffmpeg") || line.contains("ffprobe")) 
                && line.contains(parent_dir.to_str().unwrap_or(""))
                && (output_basename.is_empty() || line.contains(output_basename)) {
                // Extract container ID (first field)
                if let Some(container_id) = line.split_whitespace().next() {
                    info!("Job {}: Found Docker container {} - attempting graceful stop", job.id, container_id);
                    
                    // Try graceful stop first
                    let stop_output = Command::new(&cfg.docker_bin)
                        .arg("stop")
                        .arg("--time")
                        .arg("10")
                        .arg(container_id)
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .output()
                        .await;
                    
                    match stop_output {
                        Ok(output) if output.status.success() => {
                            info!("Job {}: ✅ Gracefully stopped container {}", job.id, container_id);
                            // Wait a bit for cleanup
                            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                            
                            // Remove container (should already be removed if --rm was used, but check)
                            let _ = Command::new(&cfg.docker_bin)
                                .arg("rm")
                                .arg(container_id)
                                .output()
                                .await;
                        }
                        Ok(output) => {
                            warn!("Job {}: Graceful stop failed for container {}: {}", 
                                  job.id, container_id, String::from_utf8_lossy(&output.stderr));
                            // Try force kill
                            let kill_output = Command::new(&cfg.docker_bin)
                                .arg("kill")
                                .arg(container_id)
                                .output()
                                .await;
                            
                            if let Ok(kill_out) = kill_output {
                                if kill_out.status.success() {
                                    info!("Job {}: ⚠️  Force killed container {}", job.id, container_id);
                                } else {
                                    warn!("Job {}: Failed to kill container {}: {}", 
                                          job.id, container_id, String::from_utf8_lossy(&kill_out.stderr));
                                }
                            }
                        }
                        Err(e) => {
                            warn!("Job {}: Failed to stop container {}: {}", job.id, container_id, e);
                        }
                    }
                }
            }
        }
    }
    
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
async fn cleanup_orphaned_temp_files(cfg: &TranscodeConfig) -> Result<usize> {
    info!("🔍 Checking for orphaned temp files...");
    
    // Load all jobs to check against
    let jobs = load_all_jobs(&cfg.job_state_dir)
        .context("Failed to load jobs for cleanup")?;
    
    // Create set of source paths that have active jobs
    let active_paths: HashSet<_> = jobs
        .iter()
        .filter(|j| matches!(j.status, JobStatus::Pending | JobStatus::Running))
        .map(|j| &j.source_path)
        .collect();
    
    let mut cleaned_count = 0;
    
    // Scan library roots for temp files
    for root in &cfg.library_roots {
        if !root.exists() {
            continue;
        }
        
        // Use walkdir in blocking task to find temp files
        let temp_files = tokio::task::spawn_blocking({
            let root = root.clone();
            move || {
                let mut temp_files = Vec::new();
                for entry in walkdir::WalkDir::new(&root)
                    .follow_links(false)
                    .into_iter()
                    .filter_map(|e| e.ok())
                {
                    let path = entry.path();
                    if path.is_file() {
                        if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                            if file_name.contains(".tmp.av1.") || file_name.ends_with(".tmp.av1.mkv") {
                                temp_files.push(path.to_path_buf());
                            }
                        }
                    }
                }
                temp_files
            }
        }).await.context("Failed to scan for temp files")?;
        
        for temp_file in temp_files {
            // Try to find corresponding source path (remove .tmp.av1.mkv extension)
            // Example: "movie.tmp.av1.mkv" -> "movie.mkv"
            let source_path = if let Some(file_name) = temp_file.file_stem().and_then(|s| s.to_str()) {
                // file_name would be "movie.tmp.av1" for "movie.tmp.av1.mkv"
                if let Some(parent) = temp_file.parent() {
                    if file_name.ends_with(".tmp.av1") {
                        // Remove ".tmp.av1" suffix
                        let base_name = &file_name[..file_name.len() - 8]; // ".tmp.av1" is 8 chars
                        parent.join(format!("{}.mkv", base_name))
                    } else {
                        // Fallback: just replace .tmp.av1 in the full file name
                        let mut source_path = temp_file.clone();
                        if let Some(file_name_str) = temp_file.file_name().and_then(|n| n.to_str()) {
                            let new_name = file_name_str.replace(".tmp.av1.", ".").replace(".tmp.av1", "");
                            source_path.set_file_name(&new_name);
                        }
                        source_path
                    }
                } else {
                    temp_file.clone()
                }
            } else {
                temp_file.clone()
            };
            
            // Check if this temp file has an active job
            let has_active_job = active_paths.contains(&source_path) || 
                                 jobs.iter().any(|j| {
                                     // Also check if temp file matches any job's expected temp path
                                     j.source_path.with_extension("tmp.av1.mkv") == temp_file
                                 });
            
            if !has_active_job {
                // Orphaned temp file - delete it
                fs::remove_file(&temp_file)
                    .with_context(|| format!("Failed to delete orphaned temp file: {}", temp_file.display()))?;
                info!("🗑️  Deleted orphaned temp file: {}", temp_file.display());
                cleaned_count += 1;
            }
        }
    }
    
    if cleaned_count > 0 {
        info!("✅ Cleaned up {} orphaned temp file(s)", cleaned_count);
    } else {
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
async fn process_job(cfg: &TranscodeConfig, job: &mut Job) -> Result<()> {
    info!("Job {}: Starting ffprobe for {}", job.id, job.source_path.display());
    
    // Step 1: Run ffprobe to get metadata
    let meta = match ffprobe::probe_file(cfg, &job.source_path).await {
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
        
        info!("Job {}: Extracted basic metadata - codec: {:?}, width: {:?}, height: {:?}, fps: {:?}", 
              job.id, job.video_codec, job.video_width, job.video_height, job.video_frame_rate);
        
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

    // Step 4: Classify source
    let decision = classifier::classify_web_source(&job.source_path, &meta.format, &meta.streams);
    job.is_web_like = decision.is_web_like();
    info!("Job {}: Source classification: {:?} (web_like: {})", job.id, decision.class, job.is_web_like);

    // Step 5: Generate temp output path
    let temp_output = job.source_path.with_extension("tmp.av1.mkv");
    info!("Job {}: Temp output will be: {}", job.id, temp_output.display());

    // Step 6: Calculate optimal quality before encoding
    // This smart calculation analyzes source properties (resolution, bitrate, codec, fps)
    // to determine the best balance between quality and file compression
    let quality = ffmpeg_docker::calculate_optimal_quality(&meta, &job.source_path);
    job.av1_quality = Some(quality);
    info!("Job {}: Calculated optimal AV1 quality: {} (balance between quality and compression)", job.id, quality);
    
    // Save job with quality setting before encoding starts
    save_job(job, &cfg.job_state_dir)?;
    
    // Step 7: Run transcoding
    info!("Job {}: Starting ffmpeg transcoding with quality setting: {}...", job.id, quality);
    let ffmpeg_result = match ffmpeg_docker::run_av1_vaapi_job(
        cfg,
        &job.source_path,
        &temp_output,
        &meta,
        &decision,
    ).await {
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

    // Store quality from ffmpeg result (should match what we calculated)
    job.av1_quality = Some(ffmpeg_result.quality_used);
    info!("Job {}: Using AV1 quality setting: {} for encoding", job.id, ffmpeg_result.quality_used);
    
    if ffmpeg_result.exit_code != 0 {
        error!("Job {}: ffmpeg failed with exit code {}", job.id, ffmpeg_result.exit_code);
        error!("Job {}: ffmpeg STDOUT: {}", job.id, ffmpeg_result.stdout);
        error!("Job {}: ffmpeg STDERR: {}", job.id, ffmpeg_result.stderr);
        let reason = format!("ffmpeg exit code {}", ffmpeg_result.exit_code);
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
    if !temp_output.exists() {
        let reason = "transcoded output file does not exist".to_string();
        sidecar::write_why_txt(&job.source_path, &reason)?;
        job.status = JobStatus::Failed;
        job.reason = Some(reason);
        job.finished_at = Some(Utc::now());
        save_job(job, &cfg.job_state_dir)?;
        return Ok(());
    }

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
    fs::rename(&temp_output, &job.source_path)
        .with_context(|| format!("Failed to replace original with transcoded file: {} -> {}", 
            temp_output.display(), job.source_path.display()))?;

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
    // Quality should already be set from ffmpeg_result, but ensure it's stored
    if job.av1_quality.is_none() {
        // Fallback: if quality wasn't set, use the one from ffmpeg result
        job.av1_quality = Some(ffmpeg_result.quality_used);
    }
    
    job.status = JobStatus::Success;
    job.output_path = Some(job.source_path.clone());
    job.new_bytes = Some(new_bytes);
    job.finished_at = Some(Utc::now());
    save_job(job, &cfg.job_state_dir)?;

    info!("Job {}: ✅ SUCCESS - Original file deleted, transcoded file in place (quality: {})", 
          job.id, job.av1_quality.unwrap_or(0));
    Ok(())
}

