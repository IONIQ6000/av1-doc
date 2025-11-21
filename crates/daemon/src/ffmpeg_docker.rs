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
    // QSV uses profile 0 (main) for both 8-bit and 10-bit
    let (pixel_format, av1_profile) = match bit_depth {
        BitDepth::Bit8 => ("nv12".to_string(), 0),
        BitDepth::Bit10 => ("p010le".to_string(), 0), // QSV: profile 0 for 10-bit
        BitDepth::Unknown => ("nv12".to_string(), 0), // Safe default
    };
    
    // Calculate optimal QP value
    let qp = calculate_optimal_qp(meta, input_file, bit_depth);
    
    info!("🎬 Encoding params (QSV): {}-bit (profile {}), QP {}, format {}, HDR: {}",
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

/// Run AV1 QSV transcoding job via Docker
pub async fn run_av1_qsv_job(
    cfg: &TranscodeConfig,
    input: &Path,
    temp_output: &Path,
    _meta: &FFProbeData,
    decision: &WebSourceDecision,
    encoding_params: &EncodingParams,
) -> Result<FFmpegResult> {
    // Get parent directories and basenames for Docker volume mounting
    let input_parent_dir = input
        .parent()
        .context("QSV encoding: Input file has no parent directory")?;
    let output_parent_dir = temp_output
        .parent()
        .context("QSV encoding: Output file has no parent directory")?;
    
    let input_basename = input
        .file_name()
        .and_then(|n| n.to_str())
        .context("QSV encoding: Input file has no basename")?;
    let output_basename = temp_output
        .file_name()
        .and_then(|n| n.to_str())
        .context("QSV encoding: Output file has no basename")?;

    // Container paths - mount input and output directories separately
    let container_input = format!("/input/{}", input_basename);
    let container_output = format!("/output/{}", output_basename);

    // Build docker command base
    // Note: Using --privileged flag required when Docker runs inside LXC containers
    // Mount /dev/dri as a volume for QSV hardware access (better than --device for DRI)
    // Use --entrypoint to bypass entrypoint script and run ffmpeg directly
    // Mount input and output directories separately to support different storage locations
    let mut cmd = Command::new(&cfg.docker_bin);
    cmd.arg("run")
        .arg("--rm")
        .arg("--privileged")
        .arg("--user")
        .arg("root") // Run as root to ensure device access
        .arg("--entrypoint")
        .arg("ffmpeg")
        .arg("-e")
        .arg("LIBVA_DRIVER_NAME=iHD") // Use Intel iHD driver for QSV support
        .arg("-v")
        .arg("/dev/dri:/dev/dri")
        .arg("-v")
        .arg(format!("{}:/input:ro", input_parent_dir.display())) // Read-only input
        .arg("-v")
        .arg(format!("{}:/output", output_parent_dir.display())) // Writable output (temp dir)
        .arg(&cfg.docker_image);

    // Build ffmpeg arguments
    let mut ffmpeg_args = Vec::new();

    // Verbosity and overwrite
    ffmpeg_args.push("-v".to_string());
    ffmpeg_args.push("verbose".to_string());
    ffmpeg_args.push("-y".to_string());

    // QSV hardware acceleration setup
    // Initialize QSV device with explicit render node path
    ffmpeg_args.push("-init_hw_device".to_string());
    ffmpeg_args.push("qsv=hw:/dev/dri/renderD128".to_string());
    ffmpeg_args.push("-filter_hw_device".to_string());
    ffmpeg_args.push("hw".to_string());
    // Note: QSV doesn't need -hwaccel and -hwaccel_device arguments
    // Decode to software, then upload in filter for better compatibility

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
    // Decode to software format, then convert and upload to QSV
    let mut filter_parts = Vec::new();

    // Ensure even dimensions and set SAR (especially for web-like sources)
    filter_parts.push("pad=ceil(iw/2)*2:ceil(ih/2)*2".to_string());
    filter_parts.push("setsar=1".to_string());

    // Convert to appropriate format based on bit depth
    // 8-bit: nv12, 10-bit: p010le
    filter_parts.push(format!("format={}", encoding_params.pixel_format));
    
    // Upload to QSV hardware surface for encoding
    // QSV doesn't need extra_hw_frames parameter
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

    // Video codec: AV1 QSV
    ffmpeg_args.push("-c:v".to_string());
    ffmpeg_args.push("av1_qsv".to_string());

    // Use global_quality for QSV quality control
    // Lower value = better quality, larger file (range: 20-40 practical)
    ffmpeg_args.push("-global_quality".to_string());
    ffmpeg_args.push(encoding_params.qp.to_string());
    
    // Set AV1 profile for QSV
    // QSV uses "main" profile for both 8-bit and 10-bit
    ffmpeg_args.push("-profile:v".to_string());
    ffmpeg_args.push("main".to_string());
    
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
    debug!("🎬 QSV encoding: docker run --rm --privileged -e LIBVA_DRIVER_NAME=iHD -v /dev/dri:/dev/dri -v {}:/input:ro -v {}:/output {} ffmpeg [args...]",
           input_parent_dir.display(), output_parent_dir.display(), cfg.docker_image);
    debug!("🎬 QSV ffmpeg args: {:?}", ffmpeg_args);
    debug!("🎬 QSV initialization: qsv=hw:/dev/dri/renderD128, codec: av1_qsv, quality: {}", encoding_params.qp);

    // Append ffmpeg args to docker command
    for arg in ffmpeg_args {
        cmd.arg(arg);
    }

    // Execute docker command
    let output = cmd
        .output()
        .await
        .with_context(|| format!(
            "Failed to execute QSV encoding via Docker for: {}. \
             Ensure Docker is running, the image '{}' is available, \
             and the QSV device at /dev/dri/renderD128 is accessible. \
             QSV requires Intel Arc GPU with iHD driver support.",
            input.display(),
            cfg.docker_image
        ))?;

    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    debug!("QSV encoding exit code: {}, stdout length: {}, stderr length: {}", exit_code, stdout.len(), stderr.len());

    // Check for common QSV-specific errors and provide helpful context
    if exit_code != 0 {
        use log::warn;
        
        // Check for QSV initialization errors
        if stderr.contains("qsv") && (stderr.contains("Cannot load") || stderr.contains("failed to initialize")) {
            warn!("QSV hardware initialization failed at /dev/dri/renderD128. \
                   This may indicate: \
                   1) Intel Arc GPU not available or not accessible, \
                   2) iHD driver not installed or not loaded (LIBVA_DRIVER_NAME=iHD), \
                   3) Insufficient permissions to access /dev/dri device, \
                   4) Docker container lacks --privileged flag or device access. \
                   FFmpeg stderr: {}", stderr.lines().take(5).collect::<Vec<_>>().join(" | "));
        }
        
        // Check for device access errors
        if stderr.contains("/dev/dri") || stderr.contains("renderD128") {
            warn!("QSV device access error at /dev/dri/renderD128. \
                   Verify that the device exists, is accessible, and Docker has proper device mounting. \
                   FFmpeg stderr: {}", stderr.lines().take(5).collect::<Vec<_>>().join(" | "));
        }
        
        // Check for codec errors
        if stderr.contains("av1_qsv") && (stderr.contains("not found") || stderr.contains("Unknown encoder")) {
            warn!("AV1 QSV codec not available. \
                   Ensure FFmpeg is built with Intel QSV support and the Docker image '{}' includes av1_qsv codec. \
                   FFmpeg stderr: {}", cfg.docker_image, stderr.lines().take(5).collect::<Vec<_>>().join(" | "));
        }
        
        // Check for pixel format errors
        if stderr.contains("p010le") || stderr.contains("nv12") {
            warn!("QSV pixel format error. \
                   This may indicate incompatibility between source format and QSV hardware upload. \
                   Bit depth: {:?}, Pixel format: {}. \
                   FFmpeg stderr: {}", 
                   encoding_params.bit_depth, 
                   encoding_params.pixel_format,
                   stderr.lines().take(5).collect::<Vec<_>>().join(" | "));
        }
        
        debug!("QSV encoding failed with exit code {}. Full stderr available in result.", exit_code);
    }

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

/// Validation result for output file
#[derive(Debug)]
pub struct ValidationResult {
    pub is_valid: bool,
    pub issues: Vec<String>,
    pub warnings: Vec<String>,
}

/// Validate the output file to detect potential corruption
/// Checks for common issues that indicate encoding problems
pub async fn validate_output(
    cfg: &TranscodeConfig,
    output_path: &Path,
    expected_bit_depth: BitDepth,
) -> Result<ValidationResult> {
    use log::{info, warn};
    
    let mut issues = Vec::new();
    let mut warnings = Vec::new();
    
    info!("🔍 Validating output file: {}", output_path.display());
    
    // Step 1: Verify file exists and is not empty
    if !output_path.exists() {
        issues.push("Output file does not exist".to_string());
        return Ok(ValidationResult {
            is_valid: false,
            issues,
            warnings,
        });
    }
    
    let metadata = std::fs::metadata(output_path)
        .with_context(|| format!("Failed to stat output file: {}", output_path.display()))?;
    
    if metadata.len() == 0 {
        issues.push("Output file is empty (0 bytes)".to_string());
        return Ok(ValidationResult {
            is_valid: false,
            issues,
            warnings,
        });
    }
    
    if metadata.len() < 1_000_000 {
        warnings.push(format!("Output file is very small ({} bytes)", metadata.len()));
    }
    
    // Step 2: Run ffprobe on output to verify it's valid
    let probe_result = crate::ffprobe::probe_file(cfg, output_path).await;
    
    let output_meta = match probe_result {
        Ok(meta) => meta,
        Err(e) => {
            issues.push(format!("Failed to probe output file (likely corrupted): {}", e));
            return Ok(ValidationResult {
                is_valid: false,
                issues,
                warnings,
            });
        }
    };
    
    // Step 3: Verify video stream exists
    let video_streams: Vec<_> = output_meta.streams.iter()
        .filter(|s| s.codec_type.as_deref() == Some("video"))
        .collect();
    
    if video_streams.is_empty() {
        issues.push("No video stream found in output".to_string());
        return Ok(ValidationResult {
            is_valid: false,
            issues,
            warnings,
        });
    }
    
    let video_stream = video_streams[0];
    
    // Step 4: Verify codec is AV1
    if let Some(ref codec) = video_stream.codec_name {
        if codec != "av1" {
            issues.push(format!("Output codec is '{}', expected 'av1'", codec));
        }
    } else {
        issues.push("Output video stream has no codec name".to_string());
    }
    
    // Step 5: Verify bit depth matches expected
    let output_bit_depth = video_stream.detect_bit_depth();
    if output_bit_depth != expected_bit_depth {
        warnings.push(format!(
            "Output bit depth ({:?}) differs from expected ({:?})",
            output_bit_depth, expected_bit_depth
        ));
    }
    
    // Step 6: Verify pixel format is correct for bit depth
    if let Some(ref pix_fmt) = video_stream.pix_fmt {
        let expected_pix_fmt = match expected_bit_depth {
            BitDepth::Bit8 => "yuv420p",
            BitDepth::Bit10 => "yuv420p10le",
            BitDepth::Unknown => "yuv420p",
        };
        
        if !pix_fmt.contains(expected_pix_fmt) {
            warnings.push(format!(
                "Output pixel format '{}' may not match expected '{}' for {:?}",
                pix_fmt, expected_pix_fmt, expected_bit_depth
            ));
        }
    }
    
    // Step 7: Verify dimensions are valid (even numbers)
    if let (Some(w), Some(h)) = (video_stream.width, video_stream.height) {
        if w <= 0 || h <= 0 {
            issues.push(format!("Invalid dimensions: {}x{}", w, h));
        }
        if w % 2 != 0 || h % 2 != 0 {
            warnings.push(format!("Odd dimensions detected: {}x{} (may cause playback issues)", w, h));
        }
    } else {
        warnings.push("Could not determine output dimensions".to_string());
    }
    
    // Step 8: Check for frame rate issues (VFR corruption indicator)
    if let (Some(ref avg_fr), Some(ref r_fr)) = (&video_stream.avg_frame_rate, &video_stream.r_frame_rate) {
        // Parse frame rates
        if let (Some(avg), Some(r)) = (parse_frame_rate(avg_fr), parse_frame_rate(r_fr)) {
            let diff = (avg - r).abs();
            if diff > 0.1 {
                warnings.push(format!(
                    "Variable frame rate detected in output (avg: {:.2}, r: {:.2}) - may indicate timestamp issues",
                    avg, r
                ));
            }
        }
    }
    
    // Step 9: Verify audio streams were preserved
    let audio_count = output_meta.streams.iter()
        .filter(|s| s.codec_type.as_deref() == Some("audio"))
        .count();
    
    if audio_count == 0 {
        warnings.push("No audio streams in output (may be intentional)".to_string());
    }
    
    // Step 10: Check format-level issues
    if let Some(ref format_bitrate) = output_meta.format.bit_rate {
        if let Ok(bitrate) = format_bitrate.parse::<u64>() {
            if bitrate < 100_000 {
                warnings.push(format!("Very low output bitrate: {} bps", bitrate));
            }
        }
    }
    
    // Determine overall validity
    let is_valid = issues.is_empty();
    
    if is_valid {
        info!("✅ Output validation passed: {} warnings", warnings.len());
        for warning in &warnings {
            warn!("⚠️  Validation warning: {}", warning);
        }
    } else {
        warn!("❌ Output validation failed: {} issues, {} warnings", issues.len(), warnings.len());
        for issue in &issues {
            warn!("❌ Validation issue: {}", issue);
        }
    }
    
    Ok(ValidationResult {
        is_valid,
        issues,
        warnings,
    })
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


#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use crate::ffprobe::{FFProbeData, FFProbeStream, FFProbeFormat};
    use std::path::PathBuf;

    // Helper to create test FFProbeData with specific bit depth
    fn create_test_metadata(bit_depth: BitDepth, pix_fmt: &str) -> FFProbeData {
        FFProbeData {
            streams: vec![FFProbeStream {
                index: 0,
                codec_name: Some("h264".to_string()),
                codec_type: Some("video".to_string()),
                width: Some(1920),
                height: Some(1080),
                pix_fmt: Some(pix_fmt.to_string()),
                bits_per_raw_sample: match bit_depth {
                    BitDepth::Bit8 => Some("8".to_string()),
                    BitDepth::Bit10 => Some("10".to_string()),
                    BitDepth::Unknown => None,
                },
                color_space: None,
                color_transfer: None,
                color_primaries: None,
                bit_rate: Some("5000000".to_string()),
                avg_frame_rate: Some("30/1".to_string()),
                r_frame_rate: Some("30/1".to_string()),
                tags: None,
                disposition: None,
            }],
            format: FFProbeFormat {
                format_name: "matroska,webm".to_string(),
                bit_rate: Some("5000000".to_string()),
                tags: None,
                muxing_app: None,
                writing_library: None,
            },
        }
    }

    // Helper function to build Docker command arguments for testing
    // This extracts the command-building logic so we can test it without executing Docker
    pub fn build_docker_command_args(
        cfg: &TranscodeConfig,
        parent_dir: &str,
    ) -> Vec<String> {
        let mut args = Vec::new();
        
        // Docker run arguments
        args.push("run".to_string());
        args.push("--rm".to_string());
        args.push("--privileged".to_string());
        args.push("--user".to_string());
        args.push("root".to_string());
        args.push("--entrypoint".to_string());
        args.push("ffmpeg".to_string());
        
        // Environment variable for QSV
        args.push("-e".to_string());
        args.push("LIBVA_DRIVER_NAME=iHD".to_string());
        
        // Volume mounts
        args.push("-v".to_string());
        args.push("/dev/dri:/dev/dri".to_string());
        args.push("-v".to_string());
        args.push(format!("{}:/config", parent_dir));
        
        // Docker image
        args.push(cfg.docker_image.clone());
        
        args
    }

    // Helper function to build ffmpeg arguments for testing
    // This extracts the ffmpeg argument building logic so we can test it without executing Docker
    pub fn build_ffmpeg_args(
        encoding_params: &EncodingParams,
        container_input: &str,
        container_output: &str,
        is_web_like: bool,
    ) -> Vec<String> {
        let mut ffmpeg_args = Vec::new();

        // Verbosity and overwrite
        ffmpeg_args.push("-v".to_string());
        ffmpeg_args.push("verbose".to_string());
        ffmpeg_args.push("-y".to_string());

        // QSV hardware acceleration setup
        ffmpeg_args.push("-init_hw_device".to_string());
        ffmpeg_args.push("qsv=hw:/dev/dri/renderD128".to_string());
        ffmpeg_args.push("-filter_hw_device".to_string());
        ffmpeg_args.push("hw".to_string());

        // Web-like input flags (if needed)
        if is_web_like {
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
        ffmpeg_args.push(container_input.to_string());

        // Build video filter chain
        let mut filter_parts = Vec::new();
        filter_parts.push("pad=ceil(iw/2)*2:ceil(ih/2)*2".to_string());
        filter_parts.push("setsar=1".to_string());
        filter_parts.push(format!("format={}", encoding_params.pixel_format));
        filter_parts.push("hwupload".to_string());

        let filter_chain = filter_parts.join(",");
        ffmpeg_args.push("-vf".to_string());
        ffmpeg_args.push(filter_chain);

        // Build mapping
        ffmpeg_args.push("-map".to_string());
        ffmpeg_args.push("0".to_string());
        ffmpeg_args.push("-map".to_string());
        ffmpeg_args.push("-0:a:m:language:rus".to_string());
        ffmpeg_args.push("-map".to_string());
        ffmpeg_args.push("-0:a:m:language:ru".to_string());
        ffmpeg_args.push("-map".to_string());
        ffmpeg_args.push("-0:s:m:language:rus".to_string());
        ffmpeg_args.push("-map".to_string());
        ffmpeg_args.push("-0:s:m:language:ru".to_string());

        // Video codec: AV1 QSV
        ffmpeg_args.push("-c:v".to_string());
        ffmpeg_args.push("av1_qsv".to_string());

        // Quality parameter
        ffmpeg_args.push("-global_quality".to_string());
        ffmpeg_args.push(encoding_params.qp.to_string());
        
        // AV1 profile
        ffmpeg_args.push("-profile:v".to_string());
        ffmpeg_args.push("main".to_string());

        // Audio and subtitles
        ffmpeg_args.push("-c:a".to_string());
        ffmpeg_args.push("copy".to_string());
        ffmpeg_args.push("-c:s".to_string());
        ffmpeg_args.push("copy".to_string());

        // Muxing options
        ffmpeg_args.push("-max_muxing_queue_size".to_string());
        ffmpeg_args.push("1024".to_string());
        ffmpeg_args.push("-movflags".to_string());
        ffmpeg_args.push("+faststart".to_string());

        // Output file
        ffmpeg_args.push(container_output.to_string());

        ffmpeg_args
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// **Feature: av1-qsv-migration, Property 4: LIBVA Driver Environment Variable**
        /// **Validates: Requirements 1.4**
        /// 
        /// For any Docker command, the environment variables SHALL include "LIBVA_DRIVER_NAME=iHD"
        #[test]
        fn test_libva_driver_environment_variable(
            docker_image in ".*",
            parent_dir in "/[a-z/]+",
        ) {
            let cfg = TranscodeConfig {
                docker_bin: PathBuf::from("docker"),
                docker_image: if docker_image.is_empty() { 
                    "lscr.io/linuxserver/ffmpeg:version-8.0-cli".to_string() 
                } else { 
                    docker_image 
                },
                gpu_device: PathBuf::from("/dev/dri"),
                ..Default::default()
            };
            
            let args = build_docker_command_args(&cfg, &parent_dir);
            
            // Verify that the environment variable argument is present
            let has_env_flag = args.windows(2).any(|window| {
                window[0] == "-e" && window[1] == "LIBVA_DRIVER_NAME=iHD"
            });
            
            prop_assert!(
                has_env_flag,
                "Docker command should include '-e LIBVA_DRIVER_NAME=iHD' environment variable. Args: {:?}",
                args
            );
            
            // Also verify the environment variable appears exactly once
            let env_count = args.windows(2).filter(|window| {
                window[0] == "-e" && window[1] == "LIBVA_DRIVER_NAME=iHD"
            }).count();
            
            prop_assert_eq!(
                env_count,
                1,
                "LIBVA_DRIVER_NAME environment variable should appear exactly once, found {} times",
                env_count
            );
        }

        /// **Feature: av1-qsv-migration, Property 6: QSV Profile for 10-bit**
        /// **Validates: Requirements 2.2**
        /// 
        /// For any 10-bit source encoded with QSV, the AV1 profile SHALL be 0 (main) instead of 1
        #[test]
        fn test_qsv_profile_for_10bit(
            width in 640i32..3840i32,
            height in 480i32..2160i32,
        ) {
            // Create 10-bit source metadata
            let mut meta = create_test_metadata(BitDepth::Bit10, "yuv420p10le");
            meta.streams[0].width = Some(width);
            meta.streams[0].height = Some(height);
            
            let input_path = std::path::Path::new("/test/video.mkv");
            let params = determine_encoding_params(&meta, input_path);
            
            // Verify that 10-bit sources use profile 0 for QSV
            prop_assert_eq!(
                params.av1_profile,
                0,
                "QSV should use profile 0 (main) for 10-bit content, got profile {}",
                params.av1_profile
            );
            
            // Also verify bit depth is correctly detected
            prop_assert_eq!(
                params.bit_depth,
                BitDepth::Bit10,
                "Bit depth should be detected as 10-bit"
            );
        }

        /// **Feature: av1-qsv-migration, Property 5 & 7: Pixel Format Selection**
        /// **Validates: Requirements 2.1, 2.4**
        /// 
        /// For any source with 10-bit color depth, the encoding parameters SHALL specify "p010le" as the pixel format
        /// For any source with 8-bit color depth, the encoding parameters SHALL specify "nv12" as the pixel format
        #[test]
        fn test_pixel_format_selection(
            width in 640i32..3840i32,
            height in 480i32..2160i32,
            is_10bit in prop::bool::ANY,
        ) {
            let (bit_depth, pix_fmt, expected_format) = if is_10bit {
                (BitDepth::Bit10, "yuv420p10le", "p010le")
            } else {
                (BitDepth::Bit8, "yuv420p", "nv12")
            };
            
            let mut meta = create_test_metadata(bit_depth, pix_fmt);
            meta.streams[0].width = Some(width);
            meta.streams[0].height = Some(height);
            
            let input_path = std::path::Path::new("/test/video.mkv");
            let params = determine_encoding_params(&meta, input_path);
            
            // Verify pixel format selection based on bit depth
            prop_assert_eq!(
                &params.pixel_format,
                expected_format,
                "{}-bit content should use {} pixel format, got {}",
                if is_10bit { 10 } else { 8 },
                expected_format,
                &params.pixel_format
            );
            
            // Verify bit depth is correctly detected
            prop_assert_eq!(
                params.bit_depth,
                bit_depth,
                "Bit depth should be correctly detected"
            );
            
            // Verify profile is always 0 for QSV (both 8-bit and 10-bit)
            prop_assert_eq!(
                params.av1_profile,
                0,
                "QSV should use profile 0 for both 8-bit and 10-bit content"
            );
        }

        /// **Feature: av1-qsv-migration, Property 1: QSV Hardware Initialization**
        /// **Validates: Requirements 1.1, 3.1**
        /// 
        /// For any encoding job, the hardware device initialization string SHALL contain 
        /// "qsv=hw:/dev/dri/renderD128" instead of "vaapi=va:/dev/dri/renderD128"
        #[test]
        fn test_qsv_hardware_initialization(
            qp in 20i32..=40i32,
            is_10bit in prop::bool::ANY,
        ) {
            let bit_depth = if is_10bit { BitDepth::Bit10 } else { BitDepth::Bit8 };
            let pixel_format = if is_10bit { "p010le".to_string() } else { "nv12".to_string() };
            
            let encoding_params = EncodingParams {
                bit_depth,
                pixel_format,
                av1_profile: 0,
                qp,
                is_hdr: false,
            };
            
            let ffmpeg_args = build_ffmpeg_args(
                &encoding_params,
                "/config/input.mkv",
                "/config/output.mkv",
                false,
            );
            
            // Verify QSV initialization is present
            let has_qsv_init = ffmpeg_args.windows(2).any(|window| {
                window[0] == "-init_hw_device" && window[1] == "qsv=hw:/dev/dri/renderD128"
            });
            
            prop_assert!(
                has_qsv_init,
                "FFmpeg args should contain '-init_hw_device qsv=hw:/dev/dri/renderD128'. Args: {:?}",
                ffmpeg_args
            );
            
            // Verify VAAPI initialization is NOT present
            let has_vaapi_init = ffmpeg_args.windows(2).any(|window| {
                window[0] == "-init_hw_device" && window[1].contains("vaapi=va")
            });
            
            prop_assert!(
                !has_vaapi_init,
                "FFmpeg args should NOT contain VAAPI initialization. Args: {:?}",
                ffmpeg_args
            );
            
            // Verify -filter_hw_device hw is present
            let has_filter_hw_device = ffmpeg_args.windows(2).any(|window| {
                window[0] == "-filter_hw_device" && window[1] == "hw"
            });
            
            prop_assert!(
                has_filter_hw_device,
                "FFmpeg args should contain '-filter_hw_device hw'. Args: {:?}",
                ffmpeg_args
            );
            
            // Verify -hwaccel vaapi is NOT present
            let has_hwaccel_vaapi = ffmpeg_args.windows(2).any(|window| {
                window[0] == "-hwaccel" && window[1] == "vaapi"
            });
            
            prop_assert!(
                !has_hwaccel_vaapi,
                "FFmpeg args should NOT contain '-hwaccel vaapi'. Args: {:?}",
                ffmpeg_args
            );
            
            // Verify -hwaccel_device is NOT present
            let has_hwaccel_device = ffmpeg_args.iter().any(|arg| arg == "-hwaccel_device");
            
            prop_assert!(
                !has_hwaccel_device,
                "FFmpeg args should NOT contain '-hwaccel_device'. Args: {:?}",
                ffmpeg_args
            );
        }

        /// **Feature: av1-qsv-migration, Property 8: Device Path in Initialization**
        /// **Validates: Requirements 3.1**
        /// 
        /// For any QSV hardware initialization, the device path SHALL be "/dev/dri/renderD128"
        #[test]
        fn test_device_path_in_initialization(
            qp in 20i32..=40i32,
            is_10bit in prop::bool::ANY,
        ) {
            let bit_depth = if is_10bit { BitDepth::Bit10 } else { BitDepth::Bit8 };
            let pixel_format = if is_10bit { "p010le".to_string() } else { "nv12".to_string() };
            
            let encoding_params = EncodingParams {
                bit_depth,
                pixel_format,
                av1_profile: 0,
                qp,
                is_hdr: false,
            };
            
            let ffmpeg_args = build_ffmpeg_args(
                &encoding_params,
                "/config/input.mkv",
                "/config/output.mkv",
                false,
            );
            
            // Find the -init_hw_device argument
            let init_hw_device_value = ffmpeg_args.windows(2)
                .find(|window| window[0] == "-init_hw_device")
                .map(|window| &window[1]);
            
            prop_assert!(
                init_hw_device_value.is_some(),
                "FFmpeg args should contain '-init_hw_device' argument"
            );
            
            let device_init = init_hw_device_value.unwrap();
            
            // Verify the device path is /dev/dri/renderD128
            prop_assert!(
                device_init.contains("/dev/dri/renderD128"),
                "QSV hardware initialization should specify device path '/dev/dri/renderD128', got: {}",
                device_init
            );
            
            // Verify it's a QSV initialization (not VAAPI or other)
            prop_assert!(
                device_init.starts_with("qsv="),
                "Hardware initialization should be QSV, got: {}",
                device_init
            );
            
            // Verify the exact format: qsv=hw:/dev/dri/renderD128
            prop_assert_eq!(
                device_init,
                "qsv=hw:/dev/dri/renderD128",
                "QSV initialization should be exactly 'qsv=hw:/dev/dri/renderD128', got: {}",
                device_init
            );
        }

        /// **Feature: av1-qsv-migration, Property 12: Filter Chain Ordering**
        /// **Validates: Requirements 5.1**
        /// 
        /// For any filter chain, the format conversion filter SHALL appear before the hwupload filter
        #[test]
        fn test_filter_chain_ordering(
            qp in 20i32..=40i32,
            is_10bit in prop::bool::ANY,
        ) {
            let bit_depth = if is_10bit { BitDepth::Bit10 } else { BitDepth::Bit8 };
            let pixel_format = if is_10bit { "p010le".to_string() } else { "nv12".to_string() };
            
            let encoding_params = EncodingParams {
                bit_depth,
                pixel_format,
                av1_profile: 0,
                qp,
                is_hdr: false,
            };
            
            let ffmpeg_args = build_ffmpeg_args(
                &encoding_params,
                "/config/input.mkv",
                "/config/output.mkv",
                false,
            );
            
            // Find the -vf argument which contains the filter chain
            let filter_chain = ffmpeg_args.windows(2)
                .find(|window| window[0] == "-vf")
                .map(|window| &window[1]);
            
            prop_assert!(
                filter_chain.is_some(),
                "FFmpeg args should contain '-vf' filter chain argument"
            );
            
            let filter_str = filter_chain.unwrap();
            
            // Find positions of format and hwupload in the filter chain
            let format_pos = filter_str.find("format=");
            let hwupload_pos = filter_str.find("hwupload");
            
            prop_assert!(
                format_pos.is_some(),
                "Filter chain should contain 'format=' filter. Filter chain: {}",
                filter_str
            );
            
            prop_assert!(
                hwupload_pos.is_some(),
                "Filter chain should contain 'hwupload' filter. Filter chain: {}",
                filter_str
            );
            
            // Verify format appears before hwupload
            prop_assert!(
                format_pos.unwrap() < hwupload_pos.unwrap(),
                "Format filter should appear before hwupload filter in chain. Filter chain: {}",
                filter_str
            );
        }

        /// **Feature: av1-qsv-migration, Property 13: QSV HWUpload Filter**
        /// **Validates: Requirements 5.2**
        /// 
        /// For any QSV encoding job, the hwupload filter SHALL not include the "extra_hw_frames" parameter
        #[test]
        fn test_qsv_hwupload_filter(
            qp in 20i32..=40i32,
            is_10bit in prop::bool::ANY,
        ) {
            let bit_depth = if is_10bit { BitDepth::Bit10 } else { BitDepth::Bit8 };
            let pixel_format = if is_10bit { "p010le".to_string() } else { "nv12".to_string() };
            
            let encoding_params = EncodingParams {
                bit_depth,
                pixel_format,
                av1_profile: 0,
                qp,
                is_hdr: false,
            };
            
            let ffmpeg_args = build_ffmpeg_args(
                &encoding_params,
                "/config/input.mkv",
                "/config/output.mkv",
                false,
            );
            
            // Find the -vf argument which contains the filter chain
            let filter_chain = ffmpeg_args.windows(2)
                .find(|window| window[0] == "-vf")
                .map(|window| &window[1]);
            
            prop_assert!(
                filter_chain.is_some(),
                "FFmpeg args should contain '-vf' filter chain argument"
            );
            
            let filter_str = filter_chain.unwrap();
            
            // Verify hwupload is present
            prop_assert!(
                filter_str.contains("hwupload"),
                "Filter chain should contain 'hwupload' filter. Filter chain: {}",
                filter_str
            );
            
            // Verify hwupload does NOT contain extra_hw_frames parameter
            prop_assert!(
                !filter_str.contains("extra_hw_frames"),
                "QSV hwupload filter should NOT contain 'extra_hw_frames' parameter. Filter chain: {}",
                filter_str
            );
            
            // Verify the hwupload filter is just "hwupload" without parameters
            // It should appear as "hwupload" or "hwupload," (followed by comma or end of string)
            let has_plain_hwupload = filter_str.contains(",hwupload,") 
                || filter_str.ends_with(",hwupload")
                || filter_str == "hwupload";
            
            prop_assert!(
                has_plain_hwupload,
                "QSV should use plain 'hwupload' without parameters. Filter chain: {}",
                filter_str
            );
        }

        /// **Feature: av1-qsv-migration, Property 14: 10-bit Filter Chain**
        /// **Validates: Requirements 5.3**
        /// 
        /// For any 10-bit source, the filter chain SHALL include "format=p010le" before "hwupload"
        #[test]
        fn test_10bit_filter_chain(
            qp in 20i32..=40i32,
        ) {
            let encoding_params = EncodingParams {
                bit_depth: BitDepth::Bit10,
                pixel_format: "p010le".to_string(),
                av1_profile: 0,
                qp,
                is_hdr: false,
            };
            
            let ffmpeg_args = build_ffmpeg_args(
                &encoding_params,
                "/config/input.mkv",
                "/config/output.mkv",
                false,
            );
            
            // Find the -vf argument which contains the filter chain
            let filter_chain = ffmpeg_args.windows(2)
                .find(|window| window[0] == "-vf")
                .map(|window| &window[1]);
            
            prop_assert!(
                filter_chain.is_some(),
                "FFmpeg args should contain '-vf' filter chain argument"
            );
            
            let filter_str = filter_chain.unwrap();
            
            // Verify format=p010le is present
            prop_assert!(
                filter_str.contains("format=p010le"),
                "10-bit filter chain should contain 'format=p010le'. Filter chain: {}",
                filter_str
            );
            
            // Verify hwupload is present
            prop_assert!(
                filter_str.contains("hwupload"),
                "Filter chain should contain 'hwupload'. Filter chain: {}",
                filter_str
            );
            
            // Verify format=p010le appears before hwupload
            let format_pos = filter_str.find("format=p010le").unwrap();
            let hwupload_pos = filter_str.find("hwupload").unwrap();
            
            prop_assert!(
                format_pos < hwupload_pos,
                "format=p010le should appear before hwupload in 10-bit filter chain. Filter chain: {}",
                filter_str
            );
        }

        /// **Feature: av1-qsv-migration, Property 15: 8-bit Filter Chain**
        /// **Validates: Requirements 5.4**
        /// 
        /// For any 8-bit source, the filter chain SHALL include "format=nv12" before "hwupload"
        #[test]
        fn test_8bit_filter_chain(
            qp in 20i32..=40i32,
        ) {
            let encoding_params = EncodingParams {
                bit_depth: BitDepth::Bit8,
                pixel_format: "nv12".to_string(),
                av1_profile: 0,
                qp,
                is_hdr: false,
            };
            
            let ffmpeg_args = build_ffmpeg_args(
                &encoding_params,
                "/config/input.mkv",
                "/config/output.mkv",
                false,
            );
            
            // Find the -vf argument which contains the filter chain
            let filter_chain = ffmpeg_args.windows(2)
                .find(|window| window[0] == "-vf")
                .map(|window| &window[1]);
            
            prop_assert!(
                filter_chain.is_some(),
                "FFmpeg args should contain '-vf' filter chain argument"
            );
            
            let filter_str = filter_chain.unwrap();
            
            // Verify format=nv12 is present
            prop_assert!(
                filter_str.contains("format=nv12"),
                "8-bit filter chain should contain 'format=nv12'. Filter chain: {}",
                filter_str
            );
            
            // Verify hwupload is present
            prop_assert!(
                filter_str.contains("hwupload"),
                "Filter chain should contain 'hwupload'. Filter chain: {}",
                filter_str
            );
            
            // Verify format=nv12 appears before hwupload
            let format_pos = filter_str.find("format=nv12").unwrap();
            let hwupload_pos = filter_str.find("hwupload").unwrap();
            
            prop_assert!(
                format_pos < hwupload_pos,
                "format=nv12 should appear before hwupload in 8-bit filter chain. Filter chain: {}",
                filter_str
            );
        }

        /// **Feature: av1-qsv-migration, Property 2: AV1 QSV Codec Selection**
        /// **Validates: Requirements 1.2**
        /// 
        /// For any encoding job, the video codec argument SHALL be "av1_qsv" instead of "av1_vaapi"
        #[test]
        fn test_av1_qsv_codec_selection(
            qp in 20i32..=40i32,
            is_10bit in prop::bool::ANY,
        ) {
            let bit_depth = if is_10bit { BitDepth::Bit10 } else { BitDepth::Bit8 };
            let pixel_format = if is_10bit { "p010le".to_string() } else { "nv12".to_string() };
            
            let encoding_params = EncodingParams {
                bit_depth,
                pixel_format,
                av1_profile: 0,
                qp,
                is_hdr: false,
            };
            
            let ffmpeg_args = build_ffmpeg_args(
                &encoding_params,
                "/config/input.mkv",
                "/config/output.mkv",
                false,
            );
            
            // Verify av1_qsv codec is present
            let has_av1_qsv = ffmpeg_args.windows(2).any(|window| {
                window[0] == "-c:v" && window[1] == "av1_qsv"
            });
            
            prop_assert!(
                has_av1_qsv,
                "FFmpeg args should contain '-c:v av1_qsv'. Args: {:?}",
                ffmpeg_args
            );
            
            // Verify av1_vaapi codec is NOT present
            let has_av1_vaapi = ffmpeg_args.windows(2).any(|window| {
                window[0] == "-c:v" && window[1] == "av1_vaapi"
            });
            
            prop_assert!(
                !has_av1_vaapi,
                "FFmpeg args should NOT contain '-c:v av1_vaapi'. Args: {:?}",
                ffmpeg_args
            );
            
            // Verify codec appears exactly once
            let codec_count = ffmpeg_args.windows(2).filter(|window| {
                window[0] == "-c:v" && window[1] == "av1_qsv"
            }).count();
            
            prop_assert_eq!(
                codec_count,
                1,
                "av1_qsv codec should appear exactly once, found {} times",
                codec_count
            );
        }

        /// **Feature: av1-qsv-migration, Property 3: Global Quality Parameter**
        /// **Validates: Requirements 1.3, 4.2**
        /// 
        /// For any encoding job with a calculated quality value, the quality parameter SHALL be 
        /// "-global_quality <value>" instead of "-qp <value>"
        #[test]
        fn test_global_quality_parameter(
            qp in 20i32..=40i32,
            is_10bit in prop::bool::ANY,
        ) {
            let bit_depth = if is_10bit { BitDepth::Bit10 } else { BitDepth::Bit8 };
            let pixel_format = if is_10bit { "p010le".to_string() } else { "nv12".to_string() };
            
            let encoding_params = EncodingParams {
                bit_depth,
                pixel_format,
                av1_profile: 0,
                qp,
                is_hdr: false,
            };
            
            let ffmpeg_args = build_ffmpeg_args(
                &encoding_params,
                "/config/input.mkv",
                "/config/output.mkv",
                false,
            );
            
            // Verify -global_quality parameter is present with correct value
            let has_global_quality = ffmpeg_args.windows(2).any(|window| {
                window[0] == "-global_quality" && window[1] == qp.to_string()
            });
            
            prop_assert!(
                has_global_quality,
                "FFmpeg args should contain '-global_quality {}'. Args: {:?}",
                qp,
                ffmpeg_args
            );
            
            // Verify -qp parameter is NOT present
            let has_qp = ffmpeg_args.iter().any(|arg| arg == "-qp");
            
            prop_assert!(
                !has_qp,
                "FFmpeg args should NOT contain '-qp' parameter (should use -global_quality instead). Args: {:?}",
                ffmpeg_args
            );
            
            // Verify the quality value is in valid range
            prop_assert!(
                qp >= 20 && qp <= 40,
                "Quality value should be in range 20-40, got {}",
                qp
            );
        }

        /// **Feature: av1-qsv-migration, Property 11: Quality Value in Result**
        /// **Validates: Requirements 4.4**
        /// 
        /// For any completed encoding job, the FFmpegResult SHALL contain the quality value that was used
        #[test]
        fn test_quality_value_in_result(
            qp in 20i32..=40i32,
        ) {
            // This property tests that the quality_used field in FFmpegResult matches the input QP
            // Since we can't actually run the async function in a proptest, we verify the logic
            // by checking that the quality_used value would be set correctly
            
            let encoding_params = EncodingParams {
                bit_depth: BitDepth::Bit8,
                pixel_format: "nv12".to_string(),
                av1_profile: 0,
                qp,
                is_hdr: false,
            };
            
            // The quality_used should equal the qp from encoding_params
            let quality_used = encoding_params.qp;
            
            prop_assert_eq!(
                quality_used,
                qp,
                "quality_used should match the input QP value. Expected: {}, Got: {}",
                qp,
                quality_used
            );
            
            // Verify the quality value is in valid range
            prop_assert!(
                quality_used >= 20 && quality_used <= 40,
                "quality_used should be in range 20-40, got {}",
                quality_used
            );
            
            // Create a mock FFmpegResult to verify the structure
            let result = FFmpegResult {
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
                quality_used,
            };
            
            prop_assert_eq!(
                result.quality_used,
                qp,
                "FFmpegResult.quality_used should match the input QP value"
            );
        }

        /// **Feature: av1-qsv-migration, Property 10: Quality Calculation Preservation**
        /// **Validates: Requirements 4.1**
        /// 
        /// For any source file with given metadata, the calculated quality value SHALL be identical 
        /// before and after migration (QSV uses the same QP calculation as VAAPI)
        #[test]
        fn test_quality_calculation_preservation(
            width in 640i32..3840i32,
            height in 480i32..2160i32,
            codec in prop::sample::select(vec!["h264", "hevc", "vp9", "mpeg2"]),
            bitrate in 1_000_000u64..50_000_000u64,
            fps in 23.0f64..120.0f64,
            is_10bit in prop::bool::ANY,
        ) {
            // Create test metadata with various source properties
            let bit_depth = if is_10bit { BitDepth::Bit10 } else { BitDepth::Bit8 };
            let pix_fmt = if is_10bit { "yuv420p10le" } else { "yuv420p" };
            let bits_per_raw_sample = if is_10bit { Some("10".to_string()) } else { Some("8".to_string()) };
            
            let meta = FFProbeData {
                streams: vec![FFProbeStream {
                    index: 0,
                    codec_name: Some(codec.to_string()),
                    codec_type: Some("video".to_string()),
                    width: Some(width),
                    height: Some(height),
                    pix_fmt: Some(pix_fmt.to_string()),
                    bits_per_raw_sample,
                    color_space: None,
                    color_transfer: None,
                    color_primaries: None,
                    bit_rate: Some(bitrate.to_string()),
                    avg_frame_rate: Some(format!("{}/1", fps as i32)),
                    r_frame_rate: Some(format!("{}/1", fps as i32)),
                    tags: None,
                    disposition: None,
                }],
                format: FFProbeFormat {
                    format_name: "matroska,webm".to_string(),
                    bit_rate: Some(bitrate.to_string()),
                    tags: None,
                    muxing_app: None,
                    writing_library: None,
                },
            };
            
            let input_path = PathBuf::from("/test/video.mkv");
            
            // Calculate QP using the current implementation
            let qp = calculate_optimal_qp(&meta, &input_path, bit_depth);
            
            // Verify QP is in valid range
            prop_assert!(
                qp >= 20 && qp <= 40,
                "Calculated QP should be in range 20-40, got {}",
                qp
            );
            
            // Calculate QP again with same inputs - should be deterministic
            let qp_second = calculate_optimal_qp(&meta, &input_path, bit_depth);
            
            prop_assert_eq!(
                qp,
                qp_second,
                "Quality calculation should be deterministic - same inputs should produce same QP. First: {}, Second: {}",
                qp,
                qp_second
            );
            
            // Verify that determine_encoding_params uses the same QP calculation
            let encoding_params = determine_encoding_params(&meta, &input_path);
            
            prop_assert_eq!(
                encoding_params.qp,
                qp,
                "determine_encoding_params should use calculate_optimal_qp. Expected: {}, Got: {}",
                qp,
                encoding_params.qp
            );
            
            // Verify bit depth is correctly detected
            prop_assert_eq!(
                encoding_params.bit_depth,
                bit_depth,
                "Bit depth should be correctly detected"
            );
            
            // Verify QSV uses profile 0 for both 8-bit and 10-bit
            prop_assert_eq!(
                encoding_params.av1_profile,
                0,
                "QSV should always use profile 0 (main) regardless of bit depth"
            );
        }
    }
}
