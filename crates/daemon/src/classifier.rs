use std::path::Path;
use crate::ffprobe::{FFProbeFormat, FFProbeStream};

/// Classification of media source type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceClass {
    WebLike,
    DiscLike,
    Unknown,
}

/// Decision about whether a source is web-like, with scoring and reasons
#[derive(Debug, Clone)]
pub struct WebSourceDecision {
    pub class: SourceClass,
    pub score: f64,
    pub reasons: Vec<String>,
}

impl WebSourceDecision {
    /// Check if this decision indicates web-like source
    pub fn is_web_like(&self) -> bool {
        self.class == SourceClass::WebLike
    }
}

/// Classify a media file as web-like, disc-like, or unknown
/// Enhanced with multiple detection signals for robust classification
pub fn classify_web_source(
    path: &Path,
    format: &FFProbeFormat,
    streams: &[FFProbeStream],
) -> WebSourceDecision {
    let mut score = 0.0;
    let mut reasons = Vec::new();

    // === SIGNAL 1: Filename-based heuristics ===
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_uppercase();

    // Check for web-related tokens (strong indicators)
    let web_tokens = ["WEB-DL", "WEBRIP", "WEB", "NF", "AMZN", "HULU", "DSNP", "ATVP", "WEBDL"];
    for token in &web_tokens {
        if filename.contains(token) {
            score += 0.35;
            reasons.push(format!("filename contains {}", token));
            break; // Only count once for filename
        }
    }

    // Check for disc-related tokens (strong indicators)
    let disc_tokens = ["BLURAY", "BDRIP", "REMUX", "BDMV", "DVD", "BLU-RAY"];
    for token in &disc_tokens {
        if filename.contains(token) {
            score -= 0.35;
            reasons.push(format!("filename contains {}", token));
            break; // Only count once for filename
        }
    }

    // === SIGNAL 2: Container format analysis ===
    let format_name = format.format_name.to_lowercase();
    if format_name.contains("mp4") || format_name.contains("mov") {
        // MP4/MOV containers are common for web sources
        score += 0.15;
        reasons.push(format!("container format: {}", format.format_name));
    }

    // === SIGNAL 3: Muxing/writing app analysis ===
    if let Some(ref muxing_app) = format.muxing_app {
        let mux_lower = muxing_app.to_lowercase();
        if mux_lower.contains("mkvmerge") || mux_lower.contains("handbrake") {
            score += 0.1;
            reasons.push(format!("muxing_app: {}", muxing_app));
        }
        // Check for disc-related muxing tools
        if mux_lower.contains("makemkv") || mux_lower.contains("anydvd") {
            score -= 0.15;
            reasons.push(format!("disc muxing_app: {}", muxing_app));
        }
    }

    if let Some(ref writing_lib) = format.writing_library {
        let lib_lower = writing_lib.to_lowercase();
        if lib_lower.contains("libmkv") {
            score += 0.1;
            reasons.push(format!("writing_library: {}", writing_lib));
        }
    }

    // === SIGNAL 4: Audio codec analysis ===
    let audio_streams: Vec<_> = streams.iter()
        .filter(|s| s.codec_type.as_deref() == Some("audio"))
        .collect();
    
    for audio_stream in &audio_streams {
        if let Some(ref codec) = audio_stream.codec_name {
            let codec_lower = codec.to_lowercase();
            // Web indicators
            if codec_lower == "aac" || codec_lower == "opus" || codec_lower == "mp3" {
                score += 0.1;
                reasons.push(format!("web audio codec: {}", codec));
                break;
            }
            // Disc indicators (lossless/high-quality codecs)
            if codec_lower == "truehd" || codec_lower == "dts" || 
               codec_lower == "flac" || codec_lower.contains("pcm") {
                score -= 0.15;
                reasons.push(format!("disc audio codec: {}", codec));
                break;
            }
            // E-AC3 is common in both, but multiple E-AC3 tracks suggest disc
            if codec_lower == "eac3" && audio_streams.len() > 2 {
                score -= 0.1;
                reasons.push("multiple eac3 tracks (disc indicator)".to_string());
                break;
            }
        }
    }

    // === SIGNAL 5: Stream count analysis ===
    let audio_count = audio_streams.len();
    let subtitle_streams: Vec<_> = streams.iter()
        .filter(|s| s.codec_type.as_deref() == Some("subtitle"))
        .collect();
    let subtitle_count = subtitle_streams.len();

    if audio_count == 1 && subtitle_count <= 2 {
        // Single audio track with few subtitles → likely web
        score += 0.1;
        reasons.push(format!("minimal streams: {} audio, {} subs (web pattern)", audio_count, subtitle_count));
    } else if audio_count >= 3 || subtitle_count >= 5 {
        // Multiple audio tracks or many subtitles → likely disc
        score -= 0.15;
        reasons.push(format!("many streams: {} audio, {} subs (disc pattern)", audio_count, subtitle_count));
    }

    // === SIGNAL 6: Video stream analysis ===
    for stream in streams {
        if stream.codec_type.as_deref() == Some("video") {
            // Check for variable frame rate (common in web sources)
            if let (Some(ref avg_fr), Some(ref r_fr)) = (stream.avg_frame_rate.as_ref(), stream.r_frame_rate.as_ref()) {
                if avg_fr != r_fr {
                    score += 0.2;
                    reasons.push("variable frame rate detected".to_string());
                }
            }

            // Check for odd dimensions (common in web sources)
            if let (Some(w), Some(h)) = (stream.width, stream.height) {
                if w % 2 != 0 || h % 2 != 0 {
                    score += 0.15;
                    reasons.push(format!("odd dimensions: {}x{}", w, h));
                }
            }

            // Check video codec and profile
            if let Some(ref _codec) = stream.codec_name {
                // Check for tags that might indicate source
                if let Some(ref tags) = stream.tags {
                    for (key, value) in tags {
                        let key_lower = key.to_lowercase();
                        let value_lower = value.to_lowercase();
                        
                        // Check for encoder strings
                        if key_lower.contains("encoder") {
                            if value_lower.contains("x264") && value_lower.contains("cabac=1") {
                                // x264 with specific settings common in web encodes
                                score += 0.05;
                                reasons.push("x264 encoder detected".to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    // === SIGNAL 7: Bitrate analysis ===
    if let Some(ref bitrate_str) = format.bit_rate {
        if let Ok(bitrate) = bitrate_str.parse::<u64>() {
            // Get video resolution for bitrate-per-pixel analysis
            let video_stream = streams.iter()
                .find(|s| s.codec_type.as_deref() == Some("video"));
            
            if let Some(vs) = video_stream {
                if let (Some(w), Some(h)) = (vs.width, vs.height) {
                    let pixels = (w * h) as f64;
                    let bitrate_per_pixel = (bitrate as f64) / pixels;
                    
                    // Web sources typically have lower bitrate per pixel
                    if bitrate_per_pixel < 0.15 {
                        score += 0.1;
                        reasons.push(format!("low bitrate/pixel: {:.4} (web pattern)", bitrate_per_pixel));
                    } else if bitrate_per_pixel > 0.3 {
                        score -= 0.1;
                        reasons.push(format!("high bitrate/pixel: {:.4} (disc pattern)", bitrate_per_pixel));
                    }
                }
            }
        }
    }

    // Determine class based on score with adjusted thresholds
    // Higher threshold for web classification to reduce false positives
    let class = if score >= 0.4 {
        SourceClass::WebLike
    } else if score <= -0.3 {
        SourceClass::DiscLike
    } else {
        SourceClass::Unknown
    };

    WebSourceDecision {
        class,
        score,
        reasons,
    }
}

