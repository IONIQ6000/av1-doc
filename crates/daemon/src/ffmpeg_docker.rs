use std::path::Path;
use anyhow::{Context, Result};
use tokio::process::Command;
use crate::config::TranscodeConfig;
use crate::ffprobe::FFProbeData;
use crate::classifier::WebSourceDecision;

/// Result of running an ffmpeg job
#[derive(Debug)]
pub struct FFmpegResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Run AV1 VAAPI transcoding job via Docker
pub async fn run_av1_vaapi_job(
    cfg: &TranscodeConfig,
    input: &Path,
    temp_output: &Path,
    meta: &FFProbeData,
    decision: &WebSourceDecision,
) -> Result<FFmpegResult> {
    // Get parent directory and basenames for Docker volume mounting
    let parent_dir = input
        .parent()
        .context("Input file has no parent directory")?;
    let input_basename = input
        .file_name()
        .and_then(|n| n.to_str())
        .context("Input file has no basename")?;
    let output_basename = temp_output
        .file_name()
        .and_then(|n| n.to_str())
        .context("Output file has no basename")?;

    // Container paths
    let container_input = format!("/config/{}", input_basename);
    let container_output = format!("/config/{}", output_basename);

    // Build docker command base
    // Note: Using --privileged flag required when Docker runs inside LXC containers
    // This is necessary for the linuxserver/ffmpeg image to set sysctls and access GPU
    let mut cmd = Command::new(&cfg.docker_bin);
    cmd.arg("run")
        .arg("--rm")
        .arg("--privileged")
        .arg("--device")
        .arg(format!("{}:{}", cfg.gpu_device.display(), cfg.gpu_device.display()))
        .arg("-v")
        .arg(format!("{}:/config", parent_dir.display()))
        .arg(&cfg.docker_image);

    // Build ffmpeg arguments
    let mut ffmpeg_args = Vec::new();

    // Verbosity and overwrite
    ffmpeg_args.push("-v".to_string());
    ffmpeg_args.push("verbose".to_string());
    ffmpeg_args.push("-y".to_string());

    // VAAPI hardware acceleration setup
    ffmpeg_args.push("-init_hw_device".to_string());
    ffmpeg_args.push("vaapi=va:/dev/dri/renderD128".to_string());
    ffmpeg_args.push("-hwaccel".to_string());
    ffmpeg_args.push("vaapi".to_string());
    ffmpeg_args.push("-hwaccel_output_format".to_string());
    ffmpeg_args.push("vaapi".to_string());

    // Web-like input flags (if needed)
    if decision.is_web_like() {
        ffmpeg_args.push("-fflags".to_string());
        ffmpeg_args.push("+genpts".to_string());
        ffmpeg_args.push("-copyts".to_string());
        ffmpeg_args.push("-start_at_zero".to_string());
        ffmpeg_args.push("-vsync".to_string());
        ffmpeg_args.push("0".to_string());
        ffmpeg_args.push("-avoid_negative_ts".to_string());
        ffmpeg_args.push("make_zero".to_string());
    }

    // Input file
    ffmpeg_args.push("-i".to_string());
    ffmpeg_args.push(container_input.clone());

    // Build video filter chain
    let mut filter_parts = Vec::new();

    // Ensure even dimensions and set SAR (especially for web-like sources)
    filter_parts.push("pad=ceil(iw/2)*2:ceil(ih/2)*2".to_string());
    filter_parts.push("setsar=1".to_string());

    // VAAPI format conversion and upload
    filter_parts.push("format=nv12|vaapi".to_string());
    filter_parts.push("hwupload".to_string());

    let filter_chain = filter_parts.join(",");
    ffmpeg_args.push("-vf".to_string());
    ffmpeg_args.push(filter_chain);

    // Build mapping: map all streams, then exclude Russian tracks
    // First, map everything
    ffmpeg_args.push("-map".to_string());
    ffmpeg_args.push("0".to_string());

    // Remove Russian audio tracks
    ffmpeg_args.push("-map".to_string());
    ffmpeg_args.push("-0:a:m:language:rus".to_string());
    ffmpeg_args.push("-map".to_string());
    ffmpeg_args.push("-0:a:m:language:ru".to_string());

    // Remove Russian subtitle tracks
    ffmpeg_args.push("-map".to_string());
    ffmpeg_args.push("-0:s:m:language:rus".to_string());
    ffmpeg_args.push("-map".to_string());
    ffmpeg_args.push("-0:s:m:language:ru".to_string());

    // Video codec: AV1 VAAPI
    ffmpeg_args.push("-c:v".to_string());
    ffmpeg_args.push("av1_vaapi".to_string());

    // Quality setting based on resolution (using quality parameter for av1_vaapi)
    if let Some(video_stream) = meta.streams.iter().find(|s| s.codec_type.as_deref() == Some("video")) {
        if let Some(height) = video_stream.height {
            let quality = resolution_to_quality(height);
            ffmpeg_args.push("-quality".to_string());
            ffmpeg_args.push(quality.to_string());
        }
    }

    // Audio: copy
    ffmpeg_args.push("-c:a".to_string());
    ffmpeg_args.push("copy".to_string());

    // Subtitles: copy
    ffmpeg_args.push("-c:s".to_string());
    ffmpeg_args.push("copy".to_string());

    // Muxing options
    ffmpeg_args.push("-max_muxing_queue_size".to_string());
    ffmpeg_args.push("1024".to_string());
    ffmpeg_args.push("-movflags".to_string());
    ffmpeg_args.push("+faststart".to_string());

    // Output file
    ffmpeg_args.push(container_output.clone());

    // Log the full command for debugging (before moving ffmpeg_args)
    use log::debug;
    debug!("ffmpeg docker command: docker run --rm --privileged --device {}:{} -v {}:/config {} ffmpeg [args...]",
           cfg.gpu_device.display(), cfg.gpu_device.display(), parent_dir.display(), cfg.docker_image);
    debug!("ffmpeg args: {:?}", ffmpeg_args);

    // Append ffmpeg args to docker command
    for arg in ffmpeg_args {
        cmd.arg(arg);
    }

    // Execute docker command
    let output = cmd
        .output()
        .await
        .with_context(|| format!("Failed to execute docker ffmpeg for: {}", input.display()))?;

    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    debug!("ffmpeg exit code: {}, stdout length: {}, stderr length: {}", exit_code, stdout.len(), stderr.len());

    Ok(FFmpegResult {
        exit_code,
        stdout,
        stderr,
    })
}

/// Determine quality parameter for av1_vaapi based on resolution height
/// Returns a quality value (lower = higher quality)
fn resolution_to_quality(height: i32) -> i32 {
    if height >= 2160 {
        24 // 4K and above
    } else if height >= 1440 {
        24 // 1440p
    } else if height >= 1080 {
        25 // 1080p
    } else {
        26 // Below 1080p
    }
}

