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
        
        let mut entry_count = 0;
        let walker = WalkDir::new(root).follow_links(false);
        for entry in walker.into_iter() {
            entry_count += 1;
            
            // Log progress every 1000 entries
            if entry_count % 1000 == 0 {
                info!("Scanned {} entries so far in {}...", entry_count, root.display());
            }
            
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    warn!("Error reading directory entry: {}", e);
                    continue;
                }
            };
            
            let path = entry.path();

            // Skip directories
            if !path.is_file() {
                continue;
            }

            files_checked += 1;
            if files_checked % 1000 == 0 {
                debug!("Checked {} files so far...", files_checked);
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

            // Check skip markers
            if sidecar::has_skip_marker(path)? {
                results.push(ScanResult::Skipped(
                    path.to_path_buf(),
                    "skip marker (.av1skip) exists".to_string(),
                ));
                continue;
            }

            // Get file size
            let metadata = std::fs::metadata(path)
                .with_context(|| format!("Failed to stat file: {}", path.display()))?;
            let size = metadata.len();

            // Check size threshold
            if size <= cfg.min_bytes {
                let reason = format!("file < {} bytes", cfg.min_bytes);
                sidecar::write_why_txt(path, &reason)?;
                results.push(ScanResult::Skipped(path.to_path_buf(), reason));
                continue;
            }

            // Stable-file check: stat twice with delay
            debug!("Checking stability for: {} ({} bytes)", path.display(), size);
            let size0 = size;
            tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
            let size1 = std::fs::metadata(path)
                .with_context(|| format!("Failed to re-stat file: {}", path.display()))?
                .len();

            if size1 != size0 {
                let reason = "file still copying".to_string();
                sidecar::write_why_txt(path, &reason)?;
                results.push(ScanResult::Skipped(path.to_path_buf(), reason));
                continue;
            }

            // This is a candidate
            info!("Found candidate: {} ({} bytes)", path.display(), size1);
            results.push(ScanResult::Candidate(path.to_path_buf(), size1));
        }
        
        info!("Finished scanning {}: {} total entries, {} files checked, {} media files found", 
              root.display(), entry_count, files_checked, media_files_found);
    }

    info!("Scan complete: checked {} files total, found {} media files, {} candidates", 
          files_checked, media_files_found, results.len());
    Ok(results)
}

