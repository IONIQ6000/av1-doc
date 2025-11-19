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
    use log::debug;
    
    // Verify file exists before trying to probe
    if !file_path.exists() {
        anyhow::bail!("File does not exist: {}", file_path.display());
    }
    
    // Get parent directory and basename for Docker volume mounting
    let parent_dir = file_path
        .parent()
        .context("File path has no parent directory")?;
    let basename = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .context("File path has no basename")?;

    // Verify parent directory exists
    if !parent_dir.exists() {
        anyhow::bail!("Parent directory does not exist: {}", parent_dir.display());
    }

    // Container path will be /config/<basename>
    // Use proper escaping for paths with spaces/special chars
    let container_path = format!("/config/{}", basename);

    debug!("ffprobe: mounting {} to /config", parent_dir.display());
    debug!("ffprobe: probing file {} in container", container_path);

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
        .arg(format!("{}:/config:ro", parent_dir.display()))
        .arg(&cfg.docker_image)
        .arg("ffprobe")
        .arg("-v")
        .arg("error")
        .arg("-print_format")
        .arg("json")
        .arg("-show_streams")
        .arg("-show_format")
        .arg(&container_path);
    
    debug!("ffprobe command: docker run --rm --privileged --device {}:{} -v {}:/config:ro {} ffprobe -v error -print_format json -show_streams -show_format {}",
           cfg.gpu_device.display(), cfg.gpu_device.display(), parent_dir.display(), cfg.docker_image, container_path);

    // Execute and capture output
    let output = cmd
        .output()
        .await
        .with_context(|| format!("Failed to execute docker ffprobe for: {}", file_path.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let exit_code = output.status.code().unwrap_or(-1);
        
        // Log the full command for debugging
        debug!("ffprobe command failed. Full command would be:");
        debug!("  docker run --rm --privileged --device {}:{} -v {}:/config:ro {} ffprobe ...",
               cfg.gpu_device.display(), cfg.gpu_device.display(),
               parent_dir.display(), cfg.docker_image);
        
        anyhow::bail!(
            "ffprobe failed (exit code {}) for {}:\nParent dir: {}\nBasename: {}\nContainer path: {}\nSTDERR: {}\nSTDOUT: {}",
            exit_code,
            file_path.display(),
            parent_dir.display(),
            basename,
            container_path,
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

