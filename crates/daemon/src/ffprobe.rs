use std::path::Path;
use std::collections::HashMap;
use anyhow::{Context, Result};
use serde::Deserialize;
use crate::config::TranscodeConfig;
use tokio::process::Command;

/// Complete ffprobe output structure
#[derive(Debug, Clone, Deserialize)]
pub struct FFProbeData {
    pub streams: Vec<FFProbeStream>,
    pub format: FFProbeFormat,
}

/// Format-level metadata from ffprobe
#[derive(Debug, Clone, Deserialize)]
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
#[derive(Debug, Clone, Deserialize)]
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
    #[serde(rename = "pix_fmt")]
    pub pix_fmt: Option<String>,
    #[serde(rename = "bits_per_raw_sample")]
    pub bits_per_raw_sample: Option<String>,
    #[serde(rename = "color_transfer")]
    pub color_transfer: Option<String>,
    #[serde(rename = "color_primaries")]
    pub color_primaries: Option<String>,
    #[serde(rename = "color_space")]
    pub color_space: Option<String>,
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
    // Use --entrypoint to bypass any entrypoint scripts that might interfere
    let mut cmd = Command::new(&cfg.docker_bin);
    cmd.arg("run")
        .arg("--rm")
        .arg("--privileged")
        .arg("--entrypoint")
        .arg("ffprobe")
        .arg("-v")
        .arg("/dev/dri:/dev/dri")
        .arg("-v")
        .arg(format!("{}:/config:ro", parent_dir.display()))
        .arg(&cfg.docker_image)
        .arg("-v")
        .arg("error")
        .arg("-print_format")
        .arg("json")
        .arg("-show_streams")
        .arg("-show_format")
        .arg(&container_path);
    
    debug!("ffprobe command: docker run --rm --privileged --entrypoint ffprobe --device {}:{} -v {}:/config:ro {} -v error -print_format json -show_streams -show_format {}",
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


/// Bit depth of video content
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitDepth {
    Bit8,
    Bit10,
    Unknown,
}

impl FFProbeStream {
    /// Detect bit depth from stream metadata
    /// Checks multiple sources: bits_per_raw_sample, pix_fmt, and HDR metadata
    pub fn detect_bit_depth(&self) -> BitDepth {
        // Method 1: Check bits_per_raw_sample (most reliable)
        if let Some(ref bits) = self.bits_per_raw_sample {
            if bits == "10" {
                return BitDepth::Bit10;
            } else if bits == "8" {
                return BitDepth::Bit8;
            }
        }
        
        // Method 2: Parse pixel format for "10" suffix
        if let Some(ref pix_fmt) = self.pix_fmt {
            let fmt_lower = pix_fmt.to_lowercase();
            if fmt_lower.contains("10") || fmt_lower.contains("p010") {
                return BitDepth::Bit10;
            }
        }
        
        // Method 3: Check HDR metadata (implies 10-bit)
        if self.is_hdr_content() {
            return BitDepth::Bit10;
        }
        
        // Default to 8-bit if unknown
        BitDepth::Bit8
    }
    
    /// Check if content is HDR (High Dynamic Range)
    /// HDR content requires 10-bit encoding
    pub fn is_hdr_content(&self) -> bool {
        // Check color transfer characteristics
        if let Some(ref transfer) = self.color_transfer {
            let t = transfer.to_lowercase();
            // PQ (Perceptual Quantizer) - HDR10
            if t.contains("smpte2084") || t.contains("st2084") {
                return true;
            }
            // HLG (Hybrid Log-Gamma) - HDR broadcast
            if t.contains("arib-std-b67") || t.contains("hlg") {
                return true;
            }
        }
        
        // Check color primaries (bt2020 often indicates HDR)
        if let Some(ref primaries) = self.color_primaries {
            let p = primaries.to_lowercase();
            if p.contains("bt2020") {
                // bt2020 with 10-bit is likely HDR
                if let Some(ref bits) = self.bits_per_raw_sample {
                    if bits == "10" {
                        return true;
                    }
                }
            }
        }
        
        false
    }
}
