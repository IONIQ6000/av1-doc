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
pub fn classify_web_source(
    path: &Path,
    format: &FFProbeFormat,
    streams: &[FFProbeStream],
) -> WebSourceDecision {
    let mut score = 0.0;
    let mut reasons = Vec::new();

    // Filename-based heuristics
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_uppercase();

    // Check for web-related tokens
    let web_tokens = ["WEB-DL", "WEBRIP", "WEB", "NF", "AMZN", "HULU", "DSNP", "ATVP"];
    for token in &web_tokens {
        if filename.contains(token) {
            score += 0.3;
            reasons.push(format!("filename contains {}", token));
        }
    }

    // Check for disc-related tokens
    let disc_tokens = ["BLURAY", "BDRIP", "REMUX", "BDMV", "DVD"];
    for token in &disc_tokens {
        if filename.contains(token) {
            score -= 0.3;
            reasons.push(format!("filename contains {}", token));
        }
    }

    // Check muxing/writing app
    if let Some(ref muxing_app) = format.muxing_app {
        let mux_lower = muxing_app.to_lowercase();
        if mux_lower.contains("mkvmerge") || mux_lower.contains("handbrake") {
            // These are common for web sources
            score += 0.1;
            reasons.push(format!("muxing_app: {}", muxing_app));
        }
    }

    if let Some(ref writing_lib) = format.writing_library {
        let lib_lower = writing_lib.to_lowercase();
        if lib_lower.contains("libmkv") {
            score += 0.1;
            reasons.push(format!("writing_library: {}", writing_lib));
        }
    }

    // Check for variable frame rate (common in web sources)
    for stream in streams {
        if stream.codec_type.as_deref() == Some("video") {
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
        }
    }

    // Determine class based on score
    let class = if score >= 0.3 {
        SourceClass::WebLike
    } else if score <= -0.2 {
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

