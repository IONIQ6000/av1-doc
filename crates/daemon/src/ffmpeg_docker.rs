use std::path::Path;
use anyhow::{Context, Result};
use tokio::process::Command;
use crate::config::TranscodeConfig;
use crate::ffprobe::{FFProbeData, BitDepth};
use crate::classifier::WebSourceDecision;

/// Result of running an ffmpeg job
#[derive(Debug)]
pub struct FFmpegResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub quality_used: i32, // QP value used for this encoding job
}

/// Encoding parameters determined from source analysis
#[derive(Debug, Clone)]
pub struct EncodingParams {
    pub bit_depth: BitDepth,
    pub pixel_format: String,
    pub av1_profile: u8,
    pub qp: i32,
    pub is_hdr: bool,
}

/// Determine optimal encoding parameters based on source analysis
pub fn determine_encoding_params(
    meta: &FFProbeData,
    input_file: &Path,
) -> EncodingParams {
    use log::info;
    
    // Find video stream
    let video_stream = meta.streams.iter()
        .find(|s| s.codec_type.as_deref() == Some("video"));
    
    // Detect bit depth
    let bit_depth = video_stream
        .map(|s| s.detect_bit_depth())
        .unwrap_or(BitDepth::Bit8);
    
    // Check for HDR
    let is_hdr = video_stream
        .map(|s| s.is_hdr_content())
        .unwrap_or(false);
    
    // Determine pixel format and AV1 profile based on bit depth
    let (pixel_format, av1_profile) = match bit_depth {
        BitDepth::Bit8 => ("nv12".to_string(), 0),
        BitDepth::Bit10 => ("p010le".to_string(), 1),
        BitDepth::Unknown => ("nv12".to_string(), 0), // Safe default
    };
    
    // Calculate optimal QP value
    let qp = calculate_optimal_qp(meta, input_file, bit_depth);
    
    info!("🎬 Encoding params: {}-bit (profile {}), QP {}, format {}, HDR: {}",
          match bit_depth {
              BitDepth::Bit8 => 8,
              BitDepth::Bit10 => 10,
              BitDepth::Unknown => 8,
          },
          av1_profile,
          qp,
          pixel_format,
          is_hdr
    );
    
    EncodingParams {
        bit_depth,
        pixel_format,
        av1_profile,
        qp,
        is_hdr,
    }
}

/// Run AV1 VAAPI transcoding job via Docker
pub async fn run_av1_vaapi_job(
    cfg: &TranscodeConfig,
    input: &Path,
    temp_output: &Path,
    meta: &FFProbeData,
    decision: &WebSourceDecision,
    encoding_params: &EncodingParams,
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

    // Convert to appropriate format based on bit depth
    // 8-bit: nv12, 10-bit: p010le
    filter_parts.push(format!("format={}", encoding_params.pixel_format));
    
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

    // Use QP (Quantization Parameter) for quality control
    // Lower QP = better quality, larger file (range: 20-40 practical)
    ffmpeg_args.push("-qp".to_string());
    ffmpeg_args.push(encoding_params.qp.to_string());
    
    // Set AV1 profile based on bit depth
    // Profile 0 (Main) = 8-bit, Profile 1 (High) = 10-bit
    ffmpeg_args.push("-profile:v".to_string());
    ffmpeg_args.push(encoding_params.av1_profile.to_string());
    
    // Store QP for return value
    let quality_used = encoding_params.qp;

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

/// Calculate optimal QP (Quantization Parameter) for AV1 encoding
/// Returns optimal QP value (range: 20-40, lower = higher quality, larger file)
/// 
/// This function analyzes source properties to determine the optimal balance
/// between quality and file compression:
/// 
/// Analysis Factors:
/// 1. **Resolution** - Higher res needs lower QP to preserve detail
/// 2. **Bit Depth** - 10-bit content needs slightly lower QP
/// 3. **Source Bitrate** - High bitrate sources can use higher QP (more compression)
/// 4. **Source Codec Efficiency** - Less efficient codecs (H.264) allow higher QP
/// 5. **Frame Rate** - Higher frame rates may need lower QP
/// 
/// QP Calculation Strategy:
/// - Base QP from resolution and bit depth
/// - Adjust based on source codec efficiency
/// - Adjust based on source bitrate efficiency
/// - Fine-tune based on frame rate
/// - Clamp to practical range (20-40)
/// 
/// Expected Results:
/// - High bitrate H.264 sources: Higher QP, more compression (~60-70% reduction)
/// - Low bitrate HEVC sources: Lower QP, preserve quality (~40-50% reduction)
/// - 4K sources: Lower QP to preserve detail (QP 26-30)
/// - 1080p sources: Balanced QP (QP 28-32)
/// - 10-bit sources: Slightly lower QP to preserve color depth
pub fn calculate_optimal_qp(
    meta: &FFProbeData,
    input_file: &Path,
    bit_depth: BitDepth,
) -> i32 {
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
    
    debug!("QP calculation inputs: resolution={}x{}, bit_depth={:?}, codec={}, fps={:.2}, video_bitrate={:?} bps, file_size={} bytes",
           width, height, bit_depth, source_codec, fps, video_bitrate_bps, file_size_bytes);
    
    // Base QP from resolution and bit depth
    // Lower QP = better quality, larger file
    let mut qp = if height >= 2160 {
        // 4K: Need lower QP to preserve detail
        if bit_depth == BitDepth::Bit10 { 26 } else { 28 }
    } else if height >= 1440 {
        // 1440p: High quality
        if bit_depth == BitDepth::Bit10 { 28 } else { 30 }
    } else if height >= 1080 {
        // 1080p: Balanced
        if bit_depth == BitDepth::Bit10 { 30 } else { 32 }
    } else {
        // 720p and below: More compression acceptable
        if bit_depth == BitDepth::Bit10 { 32 } else { 34 }
    };
    
    // FIXED: Adjust based on source codec efficiency
    // Less efficient codecs (H.264) can be compressed more aggressively
    // More efficient codecs (HEVC, VP9) should preserve quality
    let codec_adjustment = match source_codec.as_str() {
        "h264" | "avc" => {
            // H.264 is inefficient, AV1 can compress significantly more
            2 // Higher QP = more compression (CORRECTED)
        }
        "hevc" | "h265" => {
            // HEVC is already efficient, preserve quality
            -1 // Lower QP = less compression (CORRECTED)
        }
        "vp9" => {
            // VP9 is already efficient
            -1 // Lower QP = less compression
        }
        "av1" => {
            // Already AV1 - no change needed
            0
        }
        "mpeg2" | "mpeg2video" => {
            // MPEG-2 is very inefficient
            3 // Much more compression possible
        }
        _ => {
            // Unknown codec - conservative approach
            0
        }
    };
    qp += codec_adjustment;
    
    // Adjust based on source bitrate efficiency
    // Calculate bits per pixel per frame as efficiency metric
    if let Some(bitrate_bps) = video_bitrate_bps {
        let pixels = (width * height) as f64;
        let bits_per_pixel_per_frame = (bitrate_bps as f64) / (pixels * fps);
        
        debug!("Bitrate efficiency: {:.4} bpppf (bits per pixel per frame)", bits_per_pixel_per_frame);
        
        // High bitrate (inefficient encoding) = can compress more aggressively
        // Low bitrate (already efficient) = preserve quality
        if bits_per_pixel_per_frame > 0.6 {
            // Very high bitrate - compress aggressively
            qp += 3;
            debug!("Very high bitrate detected ({:.4} bpppf), increasing QP for more compression", bits_per_pixel_per_frame);
        } else if bits_per_pixel_per_frame > 0.4 {
            // High bitrate - compress more
            qp += 2;
            debug!("High bitrate detected ({:.4} bpppf), increasing QP", bits_per_pixel_per_frame);
        } else if bits_per_pixel_per_frame > 0.2 {
            // Medium bitrate - moderate compression
            qp += 1;
            debug!("Medium bitrate detected ({:.4} bpppf), slight QP increase", bits_per_pixel_per_frame);
        } else if bits_per_pixel_per_frame < 0.1 {
            // Very low bitrate - preserve quality
            qp -= 1;
            debug!("Low bitrate detected ({:.4} bpppf), decreasing QP to preserve quality", bits_per_pixel_per_frame);
        }
    }
    
    // Adjust for frame rate
    // Higher frame rates need lower QP to preserve motion detail
    if fps > 50.0 {
        qp -= 1; // Lower QP = better quality for high frame rate
        debug!("High frame rate detected ({:.2}), decreasing QP to preserve motion", fps);
    } else if fps < 24.0 {
        // Lower frame rates can use more compression
        qp += 1; // Higher QP = more compression
        debug!("Low frame rate detected ({:.2}), increasing QP", fps);
    }
    
    // Clamp QP to practical range (20-40)
    // Values below 20: Excessive quality, very large files
    // Values above 40: Noticeable quality loss
    qp = qp.max(20).min(40);
    
    // Calculate expected file size reduction for logging
    let expected_reduction_pct = calculate_expected_reduction(qp, &source_codec, bit_depth);
    
    info!("🎯 Calculated optimal QP: {} ({}x{}, {}-bit, {}, {:.2} fps, expected reduction: ~{:.0}%)",
          qp, width, height,
          match bit_depth {
              BitDepth::Bit8 => 8,
              BitDepth::Bit10 => 10,
              BitDepth::Unknown => 8,
          },
          source_codec, fps, expected_reduction_pct);
    
    qp
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

/// Calculate expected file size reduction percentage based on QP, source codec, and bit depth
fn calculate_expected_reduction(qp: i32, source_codec: &str, bit_depth: BitDepth) -> f64 {
    // Base reduction by QP value
    let base_reduction = match qp {
        20..=24 => 0.45, // ~45% reduction (very high quality)
        25..=28 => 0.55, // ~55% reduction (high quality)
        29..=32 => 0.65, // ~65% reduction (balanced)
        33..=36 => 0.72, // ~72% reduction (more compression)
        37..=40 => 0.78, // ~78% reduction (high compression)
        _ => 0.60, // Default
    };
    
    // Adjust based on source codec efficiency
    let codec_factor = match source_codec {
        "h264" | "avc" => 1.08, // H.264 allows more compression
        "hevc" | "h265" => 0.88, // HEVC already efficient, less room
        "vp9" => 0.90, // VP9 already efficient
        "mpeg2" | "mpeg2video" => 1.15, // MPEG-2 very inefficient
        _ => 1.0, // Default
    };
    
    // Adjust for bit depth (10-bit files are larger, less compression)
    let bit_depth_factor = match bit_depth {
        BitDepth::Bit10 => 0.93, // 10-bit: ~7% less compression
        _ => 1.0,
    };
    
    let reduction: f64 = base_reduction * codec_factor * bit_depth_factor * 100.0;
    reduction.min(82.0).max(30.0) // Clamp to 30-82%
}

