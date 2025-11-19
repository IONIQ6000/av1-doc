use anyhow::{Context, Result};
use clap::Parser;
use daemon::{
    config::TranscodeConfig, job::{Job, JobStatus, load_all_jobs, save_job},
    scan, ffprobe, classifier, ffmpeg_docker, sidecar,
};
use std::path::PathBuf;
use std::fs;
use std::collections::HashSet;
use chrono::Utc;
use log::{info, warn, error, debug};

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

        // Process pending jobs
        let mut jobs = load_all_jobs(&cfg.job_state_dir)
            .context("Failed to load jobs")?;

        let pending_count = jobs.iter().filter(|j| j.status == JobStatus::Pending).count();
        let running_count = jobs.iter().filter(|j| j.status == JobStatus::Running).count();

        if pending_count > 0 {
            info!("Found {} pending jobs, {} running jobs", pending_count, running_count);
        }

        // Find a pending job
        if let Some(job) = jobs.iter_mut().find(|j| j.status == JobStatus::Pending) {
            info!("Processing job {}: {}", job.id, job.source_path.display());

            job.status = JobStatus::Running;
            job.started_at = Some(Utc::now());
            save_job(job, &cfg.job_state_dir)?;

            // Process the job
            match process_job(&cfg, job).await {
                Ok(()) => {
                    info!("Job {} completed successfully", job.id);
                }
                Err(e) => {
                    error!("Job {} failed: {}", job.id, e);
                    job.status = JobStatus::Failed;
                    job.reason = Some(format!("{}", e));
                    job.finished_at = Some(Utc::now());
                    save_job(job, &cfg.job_state_dir)?;
                }
            }
        } else if pending_count == 0 && running_count == 0 {
            debug!("No pending or running jobs, waiting for next scan");
        }

        // Sleep before next scan
        info!("Sleeping for {} seconds before next scan", cfg.scan_interval_secs);
        tokio::time::sleep(tokio::time::Duration::from_secs(cfg.scan_interval_secs)).await;
    }
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
        
        // Validate all required metadata is present for estimation
        let has_codec = job.video_codec.is_some();
        let has_width = job.video_width.is_some();
        let has_height = job.video_height.is_some();
        let has_bitrate = job.video_bitrate.is_some();
        let has_fps = job.video_frame_rate.is_some();
        
        if has_codec && has_width && has_height && has_bitrate && has_fps {
            info!("Job {}: Video metadata complete - codec: {:?}, resolution: {:?}x{:?}, bitrate: {:?} bps, fps: {:?} (estimation will work)", 
                  job.id, job.video_codec, job.video_width, job.video_height, job.video_bitrate, job.video_frame_rate);
        } else {
            warn!("Job {}: Video metadata incomplete - codec: {}, width: {}, height: {}, bitrate: {}, fps: {} (estimation will NOT work)", 
                  job.id, has_codec, has_width, has_height, has_bitrate, has_fps);
            warn!("Job {}: Missing fields - codec: {:?}, width: {:?}, height: {:?}, bitrate: {:?}, fps: {:?}", 
                  job.id, job.video_codec, job.video_width, job.video_height, job.video_bitrate, job.video_frame_rate);
        }
        
        // Save job immediately after extracting metadata so TUI can use it
        save_job(job, &cfg.job_state_dir)?;
    } else {
        warn!("Job {}: No video stream found - cannot extract metadata for estimation", job.id);
    }

    // Step 4: Classify source
    let decision = classifier::classify_web_source(&job.source_path, &meta.format, &meta.streams);
    job.is_web_like = decision.is_web_like();
    info!("Job {}: Source classification: {:?} (web_like: {})", job.id, decision.class, job.is_web_like);

    // Step 5: Generate temp output path
    let temp_output = job.source_path.with_extension("tmp.av1.mkv");
    info!("Job {}: Temp output will be: {}", job.id, temp_output.display());

    // Step 6: Run transcoding
    info!("Job {}: Starting ffmpeg transcoding...", job.id);
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
            job.status = JobStatus::Failed;
            job.reason = Some(reason);
            job.finished_at = Some(Utc::now());
            save_job(job, &cfg.job_state_dir)?;
            return Ok(());
        }
    };

    if ffmpeg_result.exit_code != 0 {
        error!("Job {}: ffmpeg failed with exit code {}", job.id, ffmpeg_result.exit_code);
        error!("Job {}: ffmpeg STDOUT: {}", job.id, ffmpeg_result.stdout);
        error!("Job {}: ffmpeg STDERR: {}", job.id, ffmpeg_result.stderr);
        let reason = format!("ffmpeg exit code {}", ffmpeg_result.exit_code);
        sidecar::write_why_txt(&job.source_path, &reason)?;
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
        job.status = JobStatus::Failed;
        job.reason = Some(reason);
        job.finished_at = Some(Utc::now());
        save_job(job, &cfg.job_state_dir)?;
        return Ok(());
    }

    // Step 10: Update job status to Success - ALL CHECKS PASSED, FILE REPLACED
    job.status = JobStatus::Success;
    job.output_path = Some(job.source_path.clone());
    job.new_bytes = Some(new_bytes);
    job.finished_at = Some(Utc::now());
    save_job(job, &cfg.job_state_dir)?;

    Ok(())
}

