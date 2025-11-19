use std::path::{Path, PathBuf};
use anyhow::{Context, Result};
use std::fs;

/// Check if a skip marker (.av1skip) exists for a file
pub fn has_skip_marker(file_path: &Path) -> Result<bool> {
    let skip_path = skip_marker_path(file_path);
    Ok(skip_path.exists())
}

/// Get the path to the skip marker file for a given media file
pub fn skip_marker_path(file_path: &Path) -> PathBuf {
    let mut path = file_path.to_path_buf();
    path.set_extension("av1skip");
    path
}

/// Write a skip marker file
pub fn write_skip_marker(file_path: &Path) -> Result<()> {
    let skip_path = skip_marker_path(file_path);
    fs::write(&skip_path, "")
        .with_context(|| format!("Failed to write skip marker: {}", skip_path.display()))?;
    Ok(())
}

/// Get the path to the why.txt file for a given media file
pub fn why_txt_path(file_path: &Path) -> PathBuf {
    let mut path = file_path.to_path_buf();
    path.set_extension("why.txt");
    path
}

/// Write a why.txt file explaining why a file was skipped
pub fn write_why_txt(file_path: &Path, reason: &str) -> Result<()> {
    let why_path = why_txt_path(file_path);
    fs::write(&why_path, reason)
        .with_context(|| format!("Failed to write why.txt: {}", why_path.display()))?;
    Ok(())
}

