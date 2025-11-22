use std::path::{Path, PathBuf};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;

/// Status of a transcoding job
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Pending,
    Running,
    Success,
    Failed,
    Skipped,
}

/// Represents a transcoding job
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    /// Unique identifier for this job
    pub id: String,
    /// Path to the source media file
    pub source_path: PathBuf,
    /// Path to the output file (if completed)
    pub output_path: Option<PathBuf>,
    /// When the job was created
    pub created_at: DateTime<Utc>,
    /// When the job started processing
    pub started_at: Option<DateTime<Utc>>,
    /// When the job finished (successfully or not)
    pub finished_at: Option<DateTime<Utc>>,
    /// Current status of the job
    pub status: JobStatus,
    /// Reason for skip/failure (if applicable)
    pub reason: Option<String>,
    /// Original file size in bytes
    pub original_bytes: Option<u64>,
    /// New file size in bytes (after transcoding)
    pub new_bytes: Option<u64>,
    /// Whether the source was classified as web-like
    pub is_web_like: bool,
    /// Video codec name (e.g., "hevc", "h264")
    pub video_codec: Option<String>,
    /// Video bitrate in bits per second (from format or stream)
    pub video_bitrate: Option<u64>,
    /// Video width in pixels
    pub video_width: Option<i32>,
    /// Video height in pixels
    pub video_height: Option<i32>,
    /// Video frame rate (as fraction string, e.g., "30/1")
    pub video_frame_rate: Option<String>,
    /// AV1 encoding quality setting used (QP value, 20-40 range, lower = higher quality)
    pub av1_quality: Option<i32>,
    /// Source video bit depth (8 or 10)
    pub source_bit_depth: Option<u8>,
    /// Source pixel format (e.g., "yuv420p", "yuv420p10le")
    pub source_pix_fmt: Option<String>,
    /// Target video bit depth for encoding (8 or 10)
    pub target_bit_depth: Option<u8>,
    /// AV1 profile used (0=Main/8-bit, 1=High/10-bit)
    pub av1_profile: Option<u8>,
    /// Whether source content is HDR
    pub is_hdr: Option<bool>,
    /// Quality tier classification (Remux, WebDl, LowQuality)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality_tier: Option<String>,
    /// CRF value used for encoding (lower = higher quality)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crf_used: Option<u8>,
    /// Preset value used for encoding (lower = slower/higher quality)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset_used: Option<u8>,
    /// Encoder used (e.g., "libsvtav1", "libaom-av1", "librav1e")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoder_used: Option<String>,
    /// Path to test clip file (for REMUX sources)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test_clip_path: Option<PathBuf>,
    /// Whether test clip was approved by user
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test_clip_approved: Option<bool>,
}

impl Job {
    /// Create a new pending job
    pub fn new(source_path: PathBuf) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            source_path,
            output_path: None,
            created_at: Utc::now(),
            started_at: None,
            finished_at: None,
            status: JobStatus::Pending,
            reason: None,
            original_bytes: None,
            new_bytes: None,
            is_web_like: false,
            video_codec: None,
            video_bitrate: None,
            video_width: None,
            video_height: None,
            video_frame_rate: None,
            av1_quality: None,
            source_bit_depth: None,
            source_pix_fmt: None,
            target_bit_depth: None,
            av1_profile: None,
            is_hdr: None,
            quality_tier: None,
            crf_used: None,
            preset_used: None,
            encoder_used: None,
            test_clip_path: None,
            test_clip_approved: None,
        }
    }
}

/// Save a job to disk as a JSON file
pub fn save_job(job: &Job, dir: &Path) -> Result<()> {
    // Ensure directory exists
    fs::create_dir_all(dir)
        .with_context(|| format!("Failed to create job state directory: {}", dir.display()))?;

    let file_path = dir.join(format!("{}.json", job.id));
    let json = serde_json::to_string_pretty(job)
        .context("Failed to serialize job to JSON")?;

    fs::write(&file_path, json)
        .with_context(|| format!("Failed to write job file: {}", file_path.display()))?;

    Ok(())
}

/// Load all jobs from the job state directory
pub fn load_all_jobs(dir: &Path) -> Result<Vec<Job>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut jobs = Vec::new();

    for entry in fs::read_dir(dir)
        .with_context(|| format!("Failed to read job state directory: {}", dir.display()))?
    {
        let entry = entry.context("Failed to read directory entry")?;
        let path = entry.path();

        // Only process .json files
        if path.extension().and_then(|s| s.to_str()) == Some("json") {
            let content = fs::read_to_string(&path)
                .with_context(|| format!("Failed to read job file: {}", path.display()))?;

            let job: Job = serde_json::from_str(&content)
                .with_context(|| format!("Failed to parse job JSON: {}", path.display()))?;

            jobs.push(job);
        }
    }

    Ok(jobs)
}

