use std::path::PathBuf;
use anyhow::{Context, Result};
use walkdir::WalkDir;
use crate::config::TranscodeConfig;
use crate::sidecar;

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

    for root in &cfg.library_roots {
        if !root.exists() {
            eprintln!("Warning: Library root does not exist: {}", root.display());
            continue;
        }

        for entry in WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();

            // Skip directories
            if !path.is_file() {
                continue;
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
            results.push(ScanResult::Candidate(path.to_path_buf(), size1));
        }
    }

    Ok(results)
}

