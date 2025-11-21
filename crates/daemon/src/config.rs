use std::path::{Path, PathBuf};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Configuration for the AV1 transcoding daemon
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscodeConfig {
    /// Library root directories to scan for media files
    pub library_roots: Vec<PathBuf>,
    /// Minimum file size in bytes to consider for transcoding (e.g., 2GB)
    pub min_bytes: u64,
    /// Maximum size ratio for accepting transcoded output (e.g., 0.90 = 90% of original)
    pub max_size_ratio: f64,
    /// Directory where job state JSON files are stored
    pub job_state_dir: PathBuf,
    /// Interval in seconds between library scans
    pub scan_interval_secs: u64,
    /// Docker image to use for ffmpeg/ffprobe
    pub docker_image: String,
    /// Path to docker binary
    pub docker_bin: PathBuf,
    /// GPU device path to pass to Docker (typically /dev/dri)
    pub gpu_device: PathBuf,
    /// Directory for temporary output files (e.g., fast NVMe drive)
    /// This should be on fast storage (NVMe) separate from your media library
    pub temp_output_dir: PathBuf,
    /// Time-based timeout in seconds for stuck job detection (default: 3600 = 1 hour)
    #[serde(default = "default_stuck_job_timeout_secs")]
    pub stuck_job_timeout_secs: u64,
    /// File inactivity threshold in seconds for stuck job detection (default: 600 = 10 minutes)
    #[serde(default = "default_stuck_job_file_inactivity_secs")]
    pub stuck_job_file_inactivity_secs: u64,
    /// Enable Docker process check for stuck job detection (default: true)
    #[serde(default = "default_true")]
    pub stuck_job_check_enable_process: bool,
    /// Enable file activity check for stuck job detection (default: true)
    #[serde(default = "default_true")]
    pub stuck_job_check_enable_file_activity: bool,
    /// Directory for command files from TUI (default: {job_state_dir}/../commands)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command_dir: Option<PathBuf>,
}

fn default_stuck_job_timeout_secs() -> u64 {
    3600 // 1 hour
}

fn default_stuck_job_file_inactivity_secs() -> u64 {
    600 // 10 minutes
}

fn default_true() -> bool {
    true
}

impl Default for TranscodeConfig {
    fn default() -> Self {
        Self::default_config()
    }
}

impl TranscodeConfig {
    /// Create a default configuration with sensible values
    pub fn default_config() -> Self {
        Self {
            library_roots: vec![PathBuf::from("/media")],
            min_bytes: 2 * 1024 * 1024 * 1024, // 2GB
            max_size_ratio: 0.90,
            job_state_dir: PathBuf::from("/tmp/av1d-jobs"),
            scan_interval_secs: 60,
            docker_image: "lscr.io/linuxserver/ffmpeg:version-8.0-cli".to_string(),
            docker_bin: PathBuf::from("docker"),
            gpu_device: PathBuf::from("/dev/dri"),
            stuck_job_timeout_secs: 3600, // 1 hour
            stuck_job_file_inactivity_secs: 600, // 10 minutes
            stuck_job_check_enable_process: true,
            stuck_job_check_enable_file_activity: true,
            command_dir: None, // Will be derived from job_state_dir
            temp_output_dir: PathBuf::from("/tmp/av1d-temp"), // Fast temp storage
        }
    }
    
    /// Get the command directory path, deriving from job_state_dir if not explicitly set
    pub fn command_dir(&self) -> PathBuf {
        self.command_dir.clone().unwrap_or_else(|| {
            // Default to {job_state_dir}/../commands
            self.job_state_dir.parent()
                .map(|p| p.join("commands"))
                .unwrap_or_else(|| PathBuf::from("/var/lib/av1d/commands"))
        })
    }

    /// Load configuration from a file, or return defaults if path is None or file doesn't exist
    pub fn load_config(path: Option<&Path>) -> Result<Self> {
        let mut config = Self::default_config();

        if let Some(config_path) = path {
            if config_path.exists() {
                let content = std::fs::read_to_string(config_path)
                    .with_context(|| format!("Failed to read config file: {}", config_path.display()))?;

                // Try JSON first, then TOML
                if config_path.extension().and_then(|s| s.to_str()) == Some("toml") {
                    let file_config: TranscodeConfig = toml::from_str(&content)
                        .with_context(|| format!("Failed to parse TOML config: {}", config_path.display()))?;
                    config = file_config;
                } else {
                    let file_config: TranscodeConfig = serde_json::from_str(&content)
                        .with_context(|| format!("Failed to parse JSON config: {}", config_path.display()))?;
                    config = file_config;
                }
            }
        }

        Ok(config)
    }
}

