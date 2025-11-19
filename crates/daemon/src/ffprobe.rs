use std::path::Path;
use std::collections::HashMap;
use anyhow::{Context, Result};
use serde::Deserialize;
use crate::config::TranscodeConfig;
use tokio::process::Command;

/// Complete ffprobe output structure
#[derive(Debug, Deserialize)]
pub struct FFProbeData {
    pub streams: Vec<FFProbeStream>,
    pub format: FFProbeFormat,
}

/// Format-level metadata from ffprobe
#[derive(Debug, Deserialize)]
pub struct FFProbeFormat {
    #[serde(rename = "format_name")]
    pub format_name: String,
    #[serde(rename = "bit_rate")]
    pub bit_rate: Option<String>,
    pub tags: Option<HashMap<String, String>>,
    #[serde(rename = "muxing_app")]
    pub muxing_app: Option<String>,
    #[serde(rename = "writing_library")]
    pub writing_library: Option<String>,
}

/// Stream-level metadata from ffprobe
#[derive(Debug, Deserialize)]
pub struct FFProbeStream {
    pub index: i32,
    #[serde(rename = "codec_type")]
    pub codec_type: Option<String>,
    #[serde(rename = "codec_name")]
    pub codec_name: Option<String>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    #[serde(rename = "avg_frame_rate")]
    pub avg_frame_rate: Option<String>,
    #[serde(rename = "r_frame_rate")]
    pub r_frame_rate: Option<String>,
    pub tags: Option<HashMap<String, String>>,
    #[serde(rename = "bit_rate")]
    pub bit_rate: Option<String>,
    pub disposition: Option<HashMap<String, i32>>,
}

/// Run ffprobe via Docker and parse the JSON output
pub async fn probe_file(cfg: &TranscodeConfig, file_path: &Path) -> Result<FFProbeData> {
    // Get parent directory and basename for Docker volume mounting
    let parent_dir = file_path
        .parent()
        .context("File path has no parent directory")?;
    let basename = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .context("File path has no basename")?;

    // Container path will be /config/<basename>
    let container_path = format!("/config/{}", basename);

    // Build docker command
    // Note: Using --privileged flag required when Docker runs inside LXC containers
    // This is necessary for the linuxserver/ffmpeg image to set sysctls
    let mut cmd = Command::new(&cfg.docker_bin);
    cmd.arg("run")
        .arg("--rm")
        .arg("--privileged")
        .arg("--device")
        .arg(format!("{}:{}", cfg.gpu_device.display(), cfg.gpu_device.display()))
        .arg("-v")
        .arg(format!("{}:/config", parent_dir.display()))
        .arg(&cfg.docker_image)
        .arg("ffprobe")
        .arg("-v")
        .arg("quiet")
        .arg("-print_format")
        .arg("json")
        .arg("-show_streams")
        .arg("-show_format")
        .arg(&container_path);

    // Execute and capture output
    let output = cmd
        .output()
        .await
        .with_context(|| format!("Failed to execute docker ffprobe for: {}", file_path.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let exit_code = output.status.code().unwrap_or(-1);
        anyhow::bail!(
            "ffprobe failed (exit code {}) for {}:\nSTDERR: {}\nSTDOUT: {}",
            exit_code,
            file_path.display(),
            stderr,
            stdout
        );
    }

    // Parse JSON output
    let json_str = String::from_utf8(output.stdout)
        .context("ffprobe output is not valid UTF-8")?;

    let data: FFProbeData = serde_json::from_str(&json_str)
        .with_context(|| format!("Failed to parse ffprobe JSON for: {}", file_path.display()))?;

    Ok(data)
}

