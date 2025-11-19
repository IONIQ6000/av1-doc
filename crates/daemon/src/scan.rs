use std::path::PathBuf;
use anyhow::{Context, Result};
use walkdir::WalkDir;
use crate::config::TranscodeConfig;
use crate::sidecar;
use log::{debug, info, warn};

/// Media file extensions to consider for transcoding
const MEDIA_EXTENSIONS: &[&str] = &["mkv", "mp4", "m4v", "avi", "mov", "webm"];

/// Result of scanning a file
#[derive(Debug, Clone)]
pub enum ScanResult {
    /// File should be processed (path, size in bytes)
    Candidate(PathBuf, u64),
    /// File should be skipped (path, reason)
    Skipped(PathBuf, String),
}

/// Scan library roots for candidate media files
pub async fn scan_library(cfg: &TranscodeConfig) -> Result<Vec<ScanResult>> {
    let mut results = Vec::new();
    let mut files_checked = 0;
    let mut media_files_found = 0;

    for root in &cfg.library_roots {
        if !root.exists() {
            warn!("Library root does not exist: {}", root.display());
            continue;
        }

        info!("Scanning directory: {}", root.display());
        
        // Collect all file paths first in a blocking task to avoid blocking the async runtime
        let root_clone = root.clone();
        info!("Walking directory tree (this may take a while for large directories)...");
        let file_paths: Vec<PathBuf> = tokio::task::spawn_blocking(move || {
            let mut paths = Vec::new();
            let mut entry_count = 0;
            
            for entry in WalkDir::new(&root_clone)
                .follow_links(false)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                entry_count += 1;
                if entry_count % 10000 == 0 {
                    eprintln!("[WalkDir] Scanned {} entries...", entry_count);
                }
                
                let path = entry.path();
                if path.is_file() {
                    paths.push(path.to_path_buf());
                }
            }
            
            eprintln!("[WalkDir] Complete: {} total entries, {} files", entry_count, paths.len());
            paths
        }).await.context("Failed to scan directory")?;
        
        info!("Found {} files to check in {}", file_paths.len(), root.display());
        
        // First pass: filter to media files and check basic criteria
        let mut candidates_to_check: Vec<(PathBuf, u64)> = Vec::new();
        
        for path in file_paths {
            files_checked += 1;
            if files_checked % 100 == 0 {
                info!("Checked {} files so far in {}...", files_checked, root.display());
            }

            // Check if it's a media file by extension
            let ext = path
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| s.to_lowercase());

            if let Some(ext) = ext {
                if !MEDIA_EXTENSIONS.contains(&ext.as_str()) {
                    continue;
                }
            } else {
                continue;
            }

            media_files_found += 1;
            debug!("Found media file: {}", path.display());

            // Skip temp files from previous transcoding attempts
            let file_name = path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            if file_name.contains(".av1-tmp.") || file_name.contains(".tmp.av1.") {
                results.push(ScanResult::Skipped(
                    path.clone(),
                    "temp file from previous transcode".to_string(),
                ));
                continue;
            }

            // Check skip markers
            if sidecar::has_skip_marker(&path)? {
                results.push(ScanResult::Skipped(
                    path.clone(),
                    "skip marker (.av1skip) exists".to_string(),
                ));
                continue;
            }

            // Get file size
            let metadata = std::fs::metadata(&path)
                .with_context(|| format!("Failed to stat file: {}", path.display()))?;
            let size = metadata.len();

            // Check size threshold
            if size <= cfg.min_bytes {
                let reason = format!("file < {} bytes", cfg.min_bytes);
                sidecar::write_why_txt(&path, &reason)?;
                results.push(ScanResult::Skipped(path.clone(), reason));
                continue;
            }

            // Queue for parallel stability check
            candidates_to_check.push((path, size));
        }
        
        let total_candidates = candidates_to_check.len();
        info!("Found {} candidates requiring stability check", total_candidates);
        
        if total_candidates == 0 {
            continue;
        }
        
        // Check stability for ALL files in parallel (not sequential batches!)
        // Spawn all stability check tasks at once
        let mut all_stability_tasks = Vec::new();
        
        for (path, size) in candidates_to_check {
            let path_clone = path.clone();
            let size_clone = size;
            
            // Spawn async task to check stability - ALL tasks run concurrently
            let task = tokio::spawn(async move {
                let size0 = size_clone;
                tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
                
                match std::fs::metadata(&path_clone) {
                    Ok(metadata) => {
                        let size1 = metadata.len();
                        if size1 != size0 {
                            Some(ScanResult::Skipped(
                                path_clone,
                                "file still copying".to_string(),
                            ))
                        } else {
                            Some(ScanResult::Candidate(path_clone, size1))
                        }
                    }
                    Err(_) => None, // File disappeared, skip it
                }
            });
            
            all_stability_tasks.push(task);
        }
        
        info!("Started {} parallel stability checks (all running simultaneously)", all_stability_tasks.len());
        
        // Wait for all stability checks to complete (they're all running in parallel)
        let mut completed = 0;
        for task in all_stability_tasks {
            if let Ok(Some(result)) = task.await {
                completed += 1;
                if completed % 32 == 0 {
                    info!("Stability checks progress: {}/{} completed", completed, total_candidates);
                }
                
                match &result {
                    ScanResult::Candidate(path, size) => {
                        info!("Found candidate: {} ({} bytes)", path.display(), size);
                    }
                    ScanResult::Skipped(path, reason) => {
                        debug!("Skipped {}: {}", path.display(), reason);
                        sidecar::write_why_txt(path, reason).ok();
                    }
                }
                results.push(result);
            }
        }
        
        info!("All {} stability checks completed", completed);
        
        info!("Finished scanning {}: {} files checked, {} media files found, {} candidates", 
              root.display(), files_checked, media_files_found, 
              results.iter().filter(|r| matches!(r, ScanResult::Candidate(_, _))).count());
    }

    let candidates_count = results.iter().filter(|r| matches!(r, ScanResult::Candidate(_, _))).count();
    let skipped_count = results.len() - candidates_count;
    info!("=== SCAN COMPLETE ===");
    info!("Total files checked: {}", files_checked);
    info!("Media files found: {}", media_files_found);
    info!("Candidates: {}", candidates_count);
    info!("Skipped: {}", skipped_count);
    info!("=====================");
    Ok(results)
}
