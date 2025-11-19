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
    let args = Args::parse();

    // Load configuration
    let cfg = TranscodeConfig::load_config(args.config.as_deref())
        .context("Failed to load configuration")?;

    if args.verbose {
        println!("Configuration loaded:");
        println!("  Library roots: {:?}", cfg.library_roots);
        println!("  Min bytes: {}", cfg.min_bytes);
        println!("  Max size ratio: {}", cfg.max_size_ratio);
        println!("  Job state dir: {}", cfg.job_state_dir.display());
        println!("  Scan interval: {}s", cfg.scan_interval_secs);
    }

    // Ensure job state directory exists
    fs::create_dir_all(&cfg.job_state_dir)
        .with_context(|| format!("Failed to create job state directory: {}", cfg.job_state_dir.display()))?;

    // Main daemon loop
    loop {
        // Scan library for candidates
        if args.verbose {
            println!("Scanning library...");
        }

        let scan_results = scan::scan_library(&cfg).await
            .context("Failed to scan library")?;

        // Create jobs for new candidates
        let existing_jobs = load_all_jobs(&cfg.job_state_dir)
            .context("Failed to load existing jobs")?;

        let existing_paths: HashSet<_> = existing_jobs
            .iter()
            .map(|j| &j.source_path)
            .collect();

        for result in scan_results {
            match result {
                scan::ScanResult::Candidate(path, size) => {
                    if !existing_paths.contains(&path) {
                        let mut job = Job::new(path.clone());
                        job.original_bytes = Some(size);
                        save_job(&job, &cfg.job_state_dir)
                            .with_context(|| format!("Failed to save job for: {}", path.display()))?;

                        if args.verbose {
                            println!("Created job {} for: {}", job.id, path.display());
                        }
                    }
                }
                scan::ScanResult::Skipped(path, reason) => {
                    if args.verbose {
                        println!("Skipped {}: {}", path.display(), reason);
                    }
                }
            }
        }

        // Process pending jobs
        let mut jobs = load_all_jobs(&cfg.job_state_dir)
            .context("Failed to load jobs")?;

        // Find a pending job
        if let Some(job) = jobs.iter_mut().find(|j| j.status == JobStatus::Pending) {
            if args.verbose {
                println!("Processing job {}: {}", job.id, job.source_path.display());
            }

            job.status = JobStatus::Running;
            job.started_at = Some(Utc::now());
            save_job(job, &cfg.job_state_dir)?;

            // Process the job
            match process_job(&cfg, job).await {
                Ok(()) => {
                    if args.verbose {
                        println!("Job {} completed successfully", job.id);
                    }
                }
                Err(e) => {
                    eprintln!("Job {} failed: {}", job.id, e);
                    job.status = JobStatus::Failed;
                    job.reason = Some(format!("{}", e));
                    job.finished_at = Some(Utc::now());
                    save_job(job, &cfg.job_state_dir)?;
                }
            }
        }

        // Sleep before next scan
        tokio::time::sleep(tokio::time::Duration::from_secs(cfg.scan_interval_secs)).await;
    }
}

/// Process a single job: probe, classify, transcode, and apply size gate
async fn process_job(cfg: &TranscodeConfig, job: &mut Job) -> Result<()> {
    // Step 1: Run ffprobe to get metadata
    let meta = ffprobe::probe_file(cfg, &job.source_path).await
        .with_context(|| format!("Failed to probe file: {}", job.source_path.display()))?;

    // Step 2: Check for video streams
    let video_streams: Vec<_> = meta.streams
        .iter()
        .filter(|s| s.codec_type.as_deref() == Some("video"))
        .collect();

    if video_streams.is_empty() {
        let reason = "not a video".to_string();
        sidecar::write_why_txt(&job.source_path, &reason)?;
        job.status = JobStatus::Skipped;
        job.reason = Some(reason);
        job.finished_at = Some(Utc::now());
        save_job(job, &cfg.job_state_dir)?;
        return Ok(());
    }

    // Step 3: Check if already AV1
    if video_streams.iter().any(|s| s.codec_name.as_deref() == Some("av1")) {
        let reason = "already av1".to_string();
        sidecar::write_why_txt(&job.source_path, &reason)?;
        job.status = JobStatus::Skipped;
        job.reason = Some(reason);
        job.finished_at = Some(Utc::now());
        save_job(job, &cfg.job_state_dir)?;
        return Ok(());
    }

    // Step 4: Classify source
    let decision = classifier::classify_web_source(&job.source_path, &meta.format, &meta.streams);
    job.is_web_like = decision.is_web_like();

    // Step 5: Generate temp output path
    let temp_output = job.source_path.with_extension("tmp.av1.mkv");

    // Step 6: Run transcoding
    let ffmpeg_result = ffmpeg_docker::run_av1_vaapi_job(
        cfg,
        &job.source_path,
        &temp_output,
        &meta,
        &decision,
    ).await
        .with_context(|| format!("Failed to run ffmpeg for: {}", job.source_path.display()))?;

    if ffmpeg_result.exit_code != 0 {
        let reason = format!("ffmpeg exit code {}", ffmpeg_result.exit_code);
        sidecar::write_why_txt(&job.source_path, &reason)?;
        job.status = JobStatus::Failed;
        job.reason = Some(reason);
        job.finished_at = Some(Utc::now());
        save_job(job, &cfg.job_state_dir)?;
        return Ok(());
    }

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

