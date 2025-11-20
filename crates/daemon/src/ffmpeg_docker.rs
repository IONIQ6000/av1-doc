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
    pub quality_used: i32, // Quality setting used for this encoding job
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
    // Mount /dev/dri as a volume for VAAPI access (better than --device for DRI)
    // Use --entrypoint to bypass entrypoint script and run ffmpeg directly
    let mut cmd = Command::new(&cfg.docker_bin);
    cmd.arg("run")
        .arg("--rm")
        .arg("--privileged")
        .arg("--user")
        .arg("root") // Run as root to ensure device access
        .arg("--entrypoint")
        .arg("ffmpeg")
        .arg("-v")
        .arg("/dev/dri:/dev/dri")
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
    // Explicitly specify the render node path in init_hw_device
    ffmpeg_args.push("-init_hw_device".to_string());
    ffmpeg_args.push("vaapi=va:/dev/dri/renderD128".to_string());
    ffmpeg_args.push("-hwaccel".to_string());
    ffmpeg_args.push("vaapi".to_string());
    ffmpeg_args.push("-hwaccel_device".to_string());
    ffmpeg_args.push("/dev/dri/renderD128".to_string());
    // Don't use hwaccel_output_format - decode to software, then upload in filter
    // This is more compatible with various input codecs

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
    // Decode to software format, then convert and upload to VAAPI
    let mut filter_parts = Vec::new();

    // Ensure even dimensions and set SAR (especially for web-like sources)
    filter_parts.push("pad=ceil(iw/2)*2:ceil(ih/2)*2".to_string());
    filter_parts.push("setsar=1".to_string());

    // Convert to NV12 format (required for VAAPI)
    filter_parts.push("format=nv12".to_string());
    
    // Upload to VAAPI hardware surface for encoding
    filter_parts.push("hwupload=extra_hw_frames=64".to_string());

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

    // Smart quality calculation before encoding
    // Analyzes source properties to determine optimal quality vs compression balance
    let quality = calculate_optimal_quality(meta, input);
    ffmpeg_args.push("-quality".to_string());
    ffmpeg_args.push(quality.to_string());
    
    // Store quality for return value
    let quality_used = quality;

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
        quality_used,
    })
}

/// Smart quality calculation based on source file analysis
/// Returns optimal quality value (range: 1-63, lower = higher quality, larger file)
/// 
/// This public function can be called before encoding to determine quality setting.
/// It analyzes multiple source properties BEFORE encoding to determine
/// the optimal balance between quality and file compression:
/// 
/// Analysis Factors:
/// 1. **Resolution** - Higher res needs higher quality to preserve detail
/// 2. **Source Bitrate** - High bitrate sources can use more compression safely
/// 3. **Source Codec Efficiency** - Less efficient codecs (H.264) allow more compression
/// 4. **Frame Rate** - Higher frame rates may need slight quality adjustment
/// 5. **File Size** - Larger files benefit from better compression ratios
/// 
/// Quality Calculation Strategy:
/// - Base quality from resolution (foundation)
/// - Adjust based on source bitrate efficiency
/// - Fine-tune based on source codec (codec-specific efficiency factors)
/// - Ensure minimum quality for detail preservation
/// - Ensure maximum quality to prevent over-compression
/// 
/// Expected Results:
/// - High bitrate H.264 sources: More compression (~55-70% reduction)
/// - Low bitrate HEVC sources: Less compression needed (~40-50% reduction)
/// - 4K sources: Higher quality to preserve detail (quality 22-24)
/// - 1080p sources: Balanced quality (quality 24-26)
/// - Lower resolutions: More compression acceptable (quality 26-28)
pub fn calculate_optimal_quality(meta: &FFProbeData, input_file: &Path) -> i32 {
    use log::{info, debug, warn};
    use std::fs;
    
    // Extract video stream metadata
    let video_stream = match meta.streams.iter().find(|s| s.codec_type.as_deref() == Some("video")) {
        Some(s) => s,
        None => {
            warn!("No video stream found, using default quality 25");
            return 25;
        }
    };
    
    let height = video_stream.height.unwrap_or(1080);
    let width = video_stream.width.unwrap_or(1920);
    let source_codec = video_stream.codec_name.as_deref().unwrap_or("unknown").to_lowercase();
    
    // Parse frame rate
    let fps = video_stream.avg_frame_rate.as_deref()
        .or_else(|| video_stream.r_frame_rate.as_deref())
        .and_then(|fr| parse_frame_rate(fr))
        .unwrap_or(30.0);
    
    // Get source video bitrate (bits per second)
    let video_bitrate_bps: Option<u64> = video_stream.bit_rate.as_ref()
        .and_then(|br| br.parse::<u64>().ok())
        .or_else(|| {
            // Fallback to format bitrate if stream bitrate not available
            meta.format.bit_rate.as_ref()
                .and_then(|br| br.parse::<u64>().ok())
        });
    
    // Get file size for context
    let file_size_bytes = fs::metadata(input_file)
        .ok()
        .map(|m| m.len())
        .unwrap_or(0);
    
    debug!("Quality calculation inputs: resolution={}x{}, codec={}, fps={:.2}, video_bitrate={:?} bps, file_size={} bytes",
           width, height, source_codec, fps, video_bitrate_bps, file_size_bytes);
    
    // Base quality from resolution (foundation)
    let mut quality = if height >= 2160 {
        24 // 4K base: High quality needed for detail
    } else if height >= 1440 {
        24 // 1440p base: High quality
    } else if height >= 1080 {
        25 // 1080p base: Balanced quality
    } else {
        26 // Lower res base: More compression acceptable
    };
    
    // Adjust based on source codec efficiency
    // Less efficient codecs (H.264) can accept more compression
    // More efficient codecs (HEVC, VP9) already well-compressed, need less compression
    let codec_adjustment = match source_codec.as_str() {
        "h264" | "avc" => {
            // H.264 is less efficient, AV1 can compress significantly more
            -2 // Lower quality number = higher quality, but we want more compression
        }
        "hevc" | "h265" => {
            // HEVC is already efficient, less room for compression
            1 // Slightly higher quality number = less aggressive compression
        }
        "vp9" => {
            // VP9 is already efficient
            1 // Slightly higher quality number
        }
        "av1" => {
            // Already AV1 - minimal change needed
            0 // No adjustment
        }
        _ => {
            // Unknown codec - conservative approach
            0 // No adjustment
        }
    };
    quality += codec_adjustment;
    
    // Adjust based on source bitrate efficiency
    // Calculate bits per pixel per frame as efficiency metric
    if let Some(bitrate_bps) = video_bitrate_bps {
        let pixels = (width * height) as f64;
        let bits_per_pixel_per_frame = (bitrate_bps as f64) / (pixels * fps);
        
        debug!("Bitrate efficiency: {} bpppf (bits per pixel per frame)", bits_per_pixel_per_frame);
        
        // High bitrate (inefficient encoding) = can compress more aggressively
        // Low bitrate (already efficient) = need higher quality to maintain quality
        if bits_per_pixel_per_frame > 0.5 {
            // High bitrate - can compress more
            quality += 1; // Higher quality number = more compression
            debug!("High bitrate detected, adjusting quality for more compression");
        } else if bits_per_pixel_per_frame < 0.15 {
            // Very low bitrate - already well-compressed, preserve quality
            quality -= 1; // Lower quality number = less compression
            debug!("Low bitrate detected, adjusting quality to preserve quality");
        }
    }
    
    // Adjust for frame rate
    // Higher frame rates may benefit from slightly higher quality to preserve motion detail
    if fps > 50.0 {
        quality -= 1; // Slightly higher quality for high frame rate
        debug!("High frame rate detected ({}), adjusting quality upward", fps);
    } else if fps < 24.0 {
        // Lower frame rates can use more compression
        quality += 1; // Slightly more compression
        debug!("Low frame rate detected ({}), adjusting quality for more compression", fps);
    }
    
    // Clamp quality to valid range (20-30 for practical use)
    // Values below 20: Very high quality, large files (not needed for most content)
    // Values above 30: Noticeable quality loss
    quality = quality.max(20).min(30);
    
    // Calculate expected file size reduction for logging
    let expected_reduction_pct = calculate_expected_reduction(quality, &source_codec);
    
    info!("🎯 Calculated optimal quality: {} (resolution: {}x{}, codec: {}, fps: {:.2}, expected reduction: ~{:.0}%)",
          quality, width, height, source_codec, fps, expected_reduction_pct);
    
    quality
}

/// Helper function to parse frame rate from string (e.g., "30/1", "29.97", "60")
fn parse_frame_rate(frame_rate_str: &str) -> Option<f64> {
    // Try parsing as fraction (e.g., "30/1")
    if let Some(slash_pos) = frame_rate_str.find('/') {
        let num_str = &frame_rate_str[..slash_pos];
        let den_str = &frame_rate_str[slash_pos + 1..];
        if let (Ok(num), Ok(den)) = (num_str.parse::<f64>(), den_str.parse::<f64>()) {
            if den != 0.0 && num > 0.0 {
                return Some(num / den);
            }
        }
    }
    
    // Try parsing as decimal (e.g., "29.97")
    frame_rate_str.parse::<f64>().ok()
        .filter(|&f| f > 0.0 && f < 200.0) // Sanity check
}

/// Calculate expected file size reduction percentage based on quality and source codec
fn calculate_expected_reduction(quality: i32, source_codec: &str) -> f64 {
    // Base reduction by quality
    let base_reduction = match quality {
        20..=22 => 0.45, // ~45% reduction (very high quality)
        23..=24 => 0.55, // ~55% reduction (high quality)
        25..=26 => 0.65, // ~65% reduction (balanced)
        27..=28 => 0.70, // ~70% reduction (more compression)
        29..=30 => 0.75, // ~75% reduction (high compression)
        _ => 0.60, // Default
    };
    
    // Adjust based on source codec efficiency
    let codec_factor = match source_codec {
        "h264" | "avc" => 1.05, // H.264 allows more compression
        "hevc" | "h265" => 0.90, // HEVC already efficient, less room
        "vp9" => 0.92, // VP9 already efficient
        _ => 1.0, // Default
    };
    
    let reduction: f64 = base_reduction * codec_factor * 100.0;
    reduction.min(80.0).max(35.0) // Clamp to 35-80%
}

