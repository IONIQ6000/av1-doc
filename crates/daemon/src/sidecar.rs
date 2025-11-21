use std::path::{Path, PathBuf};
use anyhow::{Context, Result};
use std::fs;
use chrono::{DateTime, Utc};
use crate::job::Job;
use crate::classifier::WebSourceDecision;
use crate::ffmpeg_docker::{EncodingParams, ValidationResult};
use crate::ffprobe::{FFProbeData, BitDepth};

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

/// Get the path to the conversion report file for a given media file
pub fn conversion_report_path(file_path: &Path) -> PathBuf {
    let mut path = file_path.to_path_buf();
    path.set_extension("av1-conversion-report.txt");
    path
}

/// Conversion report data structure
pub struct ConversionReport {
    pub job: Job,
    pub source_meta: FFProbeData,
    pub classification: WebSourceDecision,
    pub encoding_params: EncodingParams,
    pub validation: Option<ValidationResult>,
    pub ffmpeg_stderr: Option<String>,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
}

/// Write a comprehensive conversion report file
/// This creates a detailed text file documenting every aspect of the conversion
pub fn write_conversion_report(
    file_path: &Path,
    report: &ConversionReport,
) -> Result<()> {
    let report_path = conversion_report_path(file_path);
    
    let mut content = String::new();
    
    // Header
    content.push_str("═══════════════════════════════════════════════════════════════════════════\n");
    content.push_str("                    AV1 CONVERSION REPORT (QSV)\n");
    content.push_str("═══════════════════════════════════════════════════════════════════════════\n\n");
    
    // Job Information
    content.push_str("┌─────────────────────────────────────────────────────────────────────────┐\n");
    content.push_str("│ JOB INFORMATION                                                         │\n");
    content.push_str("└─────────────────────────────────────────────────────────────────────────┘\n\n");
    
    content.push_str(&format!("Job ID:           {}\n", report.job.id));
    content.push_str(&format!("Status:           {:?}\n", report.job.status));
    content.push_str(&format!("Source File:      {}\n", report.job.source_path.display()));
    if let Some(ref output) = report.job.output_path {
        content.push_str(&format!("Output File:      {}\n", output.display()));
    }
    content.push_str(&format!("Started:          {}\n", report.start_time.format("%Y-%m-%d %H:%M:%S UTC")));
    content.push_str(&format!("Completed:        {}\n", report.end_time.format("%Y-%m-%d %H:%M:%S UTC")));
    
    let duration = report.end_time - report.start_time;
    let hours = duration.num_hours();
    let minutes = duration.num_minutes() % 60;
    let seconds = duration.num_seconds() % 60;
    content.push_str(&format!("Duration:         {}h {}m {}s\n", hours, minutes, seconds));
    content.push_str("\n");
    
    // Source Analysis
    content.push_str("┌─────────────────────────────────────────────────────────────────────────┐\n");
    content.push_str("│ SOURCE ANALYSIS                                                         │\n");
    content.push_str("└─────────────────────────────────────────────────────────────────────────┘\n\n");
    
    // Find video stream
    let video_stream = report.source_meta.streams.iter()
        .find(|s| s.codec_type.as_deref() == Some("video"));
    
    if let Some(vs) = video_stream {
        content.push_str("Video Stream:\n");
        content.push_str(&format!("  Codec:          {}\n", vs.codec_name.as_deref().unwrap_or("unknown")));
        content.push_str(&format!("  Resolution:     {}x{}\n", 
            vs.width.unwrap_or(0), vs.height.unwrap_or(0)));
        content.push_str(&format!("  Pixel Format:   {}\n", vs.pix_fmt.as_deref().unwrap_or("unknown")));
        
        let bit_depth = vs.detect_bit_depth();
        content.push_str(&format!("  Bit Depth:      {:?} ({})\n", 
            bit_depth,
            vs.bits_per_raw_sample.as_deref().unwrap_or("not specified")));
        
        let is_hdr = vs.is_hdr_content();
        content.push_str(&format!("  HDR:            {}\n", if is_hdr { "Yes" } else { "No" }));
        
        if is_hdr {
            if let Some(ref transfer) = vs.color_transfer {
                content.push_str(&format!("  Color Transfer: {}\n", transfer));
            }
            if let Some(ref primaries) = vs.color_primaries {
                content.push_str(&format!("  Color Primaries: {}\n", primaries));
            }
            if let Some(ref space) = vs.color_space {
                content.push_str(&format!("  Color Space:    {}\n", space));
            }
        }
        
        if let Some(ref fps) = vs.avg_frame_rate {
            content.push_str(&format!("  Frame Rate:     {} fps\n", fps));
        }
        if let Some(ref bitrate) = vs.bit_rate {
            if let Ok(br) = bitrate.parse::<u64>() {
                content.push_str(&format!("  Bitrate:        {:.2} Mbps\n", br as f64 / 1_000_000.0));
            }
        }
    }
    
    // Audio streams
    let audio_streams: Vec<_> = report.source_meta.streams.iter()
        .filter(|s| s.codec_type.as_deref() == Some("audio"))
        .collect();
    
    content.push_str(&format!("\nAudio Streams:    {} track(s)\n", audio_streams.len()));
    for (i, audio) in audio_streams.iter().enumerate() {
        content.push_str(&format!("  Track {}:        {} ", i + 1, 
            audio.codec_name.as_deref().unwrap_or("unknown")));
        
        if let Some(ref tags) = audio.tags {
            if let Some(lang) = tags.get("language").or_else(|| tags.get("LANGUAGE")) {
                content.push_str(&format!("({})", lang));
            }
        }
        content.push_str("\n");
    }
    
    // Subtitle streams
    let subtitle_streams: Vec<_> = report.source_meta.streams.iter()
        .filter(|s| s.codec_type.as_deref() == Some("subtitle"))
        .collect();
    
    content.push_str(&format!("\nSubtitle Streams: {} track(s)\n", subtitle_streams.len()));
    for (i, sub) in subtitle_streams.iter().enumerate() {
        content.push_str(&format!("  Track {}:        {} ", i + 1,
            sub.codec_name.as_deref().unwrap_or("unknown")));
        
        if let Some(ref tags) = sub.tags {
            if let Some(lang) = tags.get("language").or_else(|| tags.get("LANGUAGE")) {
                content.push_str(&format!("({})", lang));
            }
        }
        content.push_str("\n");
    }
    
    // Container info
    content.push_str(&format!("\nContainer:        {}\n", report.source_meta.format.format_name));
    if let Some(ref muxing) = report.source_meta.format.muxing_app {
        content.push_str(&format!("Muxing App:       {}\n", muxing));
    }
    if let Some(ref writing) = report.source_meta.format.writing_library {
        content.push_str(&format!("Writing Library:  {}\n", writing));
    }
    content.push_str("\n");
    
    // Source Classification
    content.push_str("┌─────────────────────────────────────────────────────────────────────────┐\n");
    content.push_str("│ SOURCE CLASSIFICATION                                                   │\n");
    content.push_str("└─────────────────────────────────────────────────────────────────────────┘\n\n");
    
    content.push_str(&format!("Classification:   {:?}\n", report.classification.class));
    content.push_str(&format!("Confidence Score: {:.2}\n", report.classification.score));
    content.push_str("\nDetection Signals:\n");
    for reason in &report.classification.reasons {
        content.push_str(&format!("  • {}\n", reason));
    }
    
    content.push_str("\nEncoding Strategy:\n");
    match report.classification.class {
        crate::classifier::SourceClass::WebLike => {
            content.push_str("  → WEB encoding strategy applied\n");
            content.push_str("  → Variable frame rate (VFR) handling enabled\n");
            content.push_str("  → Timestamp correction flags applied\n");
            content.push_str("  → FFmpeg flags: -fflags +genpts -copyts -start_at_zero -vsync 0\n");
        }
        crate::classifier::SourceClass::DiscLike => {
            content.push_str("  → DISC encoding strategy applied\n");
            content.push_str("  → Standard constant frame rate (CFR) processing\n");
            content.push_str("  → No special timestamp handling needed\n");
        }
        crate::classifier::SourceClass::Unknown => {
            content.push_str("  → CONSERVATIVE encoding strategy applied\n");
            content.push_str("  → Standard processing with safety margins\n");
        }
    }
    content.push_str("\n");
    
    // Encoding Parameters
    content.push_str("┌─────────────────────────────────────────────────────────────────────────┐\n");
    content.push_str("│ ENCODING PARAMETERS                                                     │\n");
    content.push_str("└─────────────────────────────────────────────────────────────────────────┘\n\n");
    
    content.push_str("Hardware Encoder: Intel QSV (Quick Sync Video)\n");
    content.push_str("Codec:            av1_qsv\n");
    content.push_str("Device:           /dev/dri/renderD128\n");
    content.push_str("Driver:           iHD (Intel Media Driver)\n");
    content.push_str("\n");
    
    content.push_str(&format!("Target Bit Depth: {:?}\n", report.encoding_params.bit_depth));
    content.push_str(&format!("Pixel Format:     {}\n", report.encoding_params.pixel_format));
    content.push_str(&format!("AV1 Profile:      {} (main)\n", report.encoding_params.av1_profile));
    content.push_str(&format!("Quality (QP):     {} (lower = higher quality)\n", report.encoding_params.qp));
    content.push_str(&format!("HDR Encoding:     {}\n", if report.encoding_params.is_hdr { "Yes" } else { "No" }));
    
    content.push_str("\nFilter Chain:\n");
    content.push_str("  1. pad=ceil(iw/2)*2:ceil(ih/2)*2  (ensure even dimensions)\n");
    content.push_str("  2. setsar=1                        (set sample aspect ratio)\n");
    content.push_str(&format!("  3. format={}                   (pixel format conversion)\n", 
        report.encoding_params.pixel_format));
    content.push_str("  4. hwupload                        (upload to GPU memory)\n");
    
    content.push_str("\nStream Handling:\n");
    content.push_str("  • Video:     Transcoded to AV1 (QSV hardware acceleration)\n");
    content.push_str("  • Audio:     Copied (no re-encoding)\n");
    content.push_str("  • Subtitles: Copied (no re-encoding)\n");
    content.push_str("  • Chapters:  Preserved\n");
    content.push_str("  • Metadata:  Preserved\n");
    content.push_str("  • Russian tracks: Removed (audio & subtitles)\n");
    content.push_str("\n");
    
    // File Size Comparison
    content.push_str("┌─────────────────────────────────────────────────────────────────────────┐\n");
    content.push_str("│ FILE SIZE COMPARISON                                                    │\n");
    content.push_str("└─────────────────────────────────────────────────────────────────────────┘\n\n");
    
    if let (Some(orig), Some(new)) = (report.job.original_bytes, report.job.new_bytes) {
        let orig_gb = orig as f64 / 1_000_000_000.0;
        let new_gb = new as f64 / 1_000_000_000.0;
        let saved_gb = orig_gb - new_gb;
        let reduction_pct = (saved_gb / orig_gb) * 100.0;
        
        content.push_str(&format!("Original Size:    {:.2} GB\n", orig_gb));
        content.push_str(&format!("New Size:         {:.2} GB\n", new_gb));
        content.push_str(&format!("Space Saved:      {:.2} GB ({:.1}% reduction)\n", saved_gb, reduction_pct));
        content.push_str(&format!("Compression:      {:.2}x smaller\n", orig_gb / new_gb));
    }
    content.push_str("\n");
    
    // Output Validation
    if let Some(ref validation) = report.validation {
        content.push_str("┌─────────────────────────────────────────────────────────────────────────┐\n");
        content.push_str("│ OUTPUT VALIDATION                                                       │\n");
        content.push_str("└─────────────────────────────────────────────────────────────────────────┘\n\n");
        
        content.push_str(&format!("Validation Status: {}\n", 
            if validation.is_valid { "✓ PASSED" } else { "✗ FAILED" }));
        content.push_str("\n");
        
        if !validation.issues.is_empty() {
            content.push_str("Issues Found:\n");
            for issue in &validation.issues {
                content.push_str(&format!("  ✗ {}\n", issue));
            }
            content.push_str("\n");
        }
        
        if !validation.warnings.is_empty() {
            content.push_str("Warnings:\n");
            for warning in &validation.warnings {
                content.push_str(&format!("  ⚠ {}\n", warning));
            }
            content.push_str("\n");
        }
        
        if validation.is_valid && validation.warnings.is_empty() {
            content.push_str("All validation checks passed:\n");
            content.push_str("  ✓ File exists and is not empty\n");
            content.push_str("  ✓ FFprobe can read file (not corrupted)\n");
            content.push_str("  ✓ Video stream exists and is valid\n");
            content.push_str("  ✓ Codec is AV1 as expected\n");
            content.push_str("  ✓ Bit depth matches target\n");
            content.push_str("  ✓ Pixel format is correct\n");
            content.push_str("  ✓ Dimensions are valid\n");
            content.push_str("  ✓ Frame rate is consistent (no VFR corruption)\n");
            content.push_str("  ✓ Audio streams preserved\n");
            content.push_str("  ✓ Bitrate is within expected range\n");
            content.push_str("\n");
        }
    }
    
    // FFmpeg Output (if available)
    if let Some(ref stderr) = report.ffmpeg_stderr {
        content.push_str("┌─────────────────────────────────────────────────────────────────────────┐\n");
        content.push_str("│ FFMPEG ENCODING LOG (Last 50 lines)                                    │\n");
        content.push_str("└─────────────────────────────────────────────────────────────────────────┘\n\n");
        
        // Get last 50 lines of stderr
        let lines: Vec<&str> = stderr.lines().collect();
        let start = if lines.len() > 50 { lines.len() - 50 } else { 0 };
        for line in &lines[start..] {
            content.push_str(line);
            content.push_str("\n");
        }
        content.push_str("\n");
    }
    
    // Footer
    content.push_str("═══════════════════════════════════════════════════════════════════════════\n");
    content.push_str("                         END OF REPORT\n");
    content.push_str("═══════════════════════════════════════════════════════════════════════════\n");
    content.push_str(&format!("\nReport generated: {}\n", Utc::now().format("%Y-%m-%d %H:%M:%S UTC")));
    content.push_str("Converter: AV1 Daemon (QSV)\n");
    content.push_str("For issues or questions, check the logs or documentation.\n");
    
    fs::write(&report_path, content)
        .with_context(|| format!("Failed to write conversion report: {}", report_path.display()))?;
    
    Ok(())
}

