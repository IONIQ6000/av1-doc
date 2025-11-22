use std::path::Path;
use crate::ffprobe::{FFProbeFormat, FFProbeStream};

/// Classification of media source type (legacy)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceClass {
    WebLike,
    DiscLike,
    Unknown,
}

/// Quality tier classification for software AV1 encoding
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualityTier {
    /// Blu-ray/UHD remux, high-bitrate masters - preserve everything
    Remux,
    /// Streaming downloads, already encoded - conservative re-encoding
    WebDl,
    /// Low-bitrate rips, already degraded - size reduction OK
    LowQuality,
}

/// Decision about whether a source is web-like, with scoring and reasons (legacy)
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

/// Enhanced source classification with quality tier and confidence
#[derive(Debug, Clone)]
pub struct SourceClassification {
    pub tier: QualityTier,
    pub confidence: f64,
    pub reasons: Vec<String>,
    pub bitrate_per_pixel: Option<f64>,
}

/// Source classifier for determining quality tier
pub struct SourceClassifier;

impl SourceClassifier {
    /// Create a new source classifier
    pub fn new() -> Self {
        SourceClassifier
    }

    /// Classify source into quality tier based on bitrate, codec, and metadata
    pub fn classify(&self, path: &Path, format: &FFProbeFormat, streams: &[FFProbeStream]) -> SourceClassification {
        use log::info;
        
        info!("🔍 Classifying source: {}", path.display());
        
        let mut reasons = Vec::new();
        let mut remux_score: f64 = 0.0;
        let mut webdl_score: f64 = 0.0;
        let mut lowquality_score: f64 = 0.0;

        // Get video stream for analysis
        let video_stream = streams.iter()
            .find(|s| s.codec_type.as_deref() == Some("video"));

        // Calculate bitrate per pixel if possible
        let bitrate_per_pixel = self.calculate_bitrate_per_pixel(format, video_stream);

        // === SIGNAL 1: Bitrate analysis (strongest indicator) ===
        if let Some(bpp) = bitrate_per_pixel {
            if let Some(vs) = video_stream {
                if let (Some(width), Some(height)) = (vs.width, vs.height) {
                    let is_1080p = height >= 1000 && height <= 1200;
                    let is_2160p = height >= 2000 && height <= 2400;

                    // Get absolute bitrate for threshold checks
                    if let Some(ref bitrate_str) = format.bit_rate {
                        if let Ok(bitrate) = bitrate_str.parse::<u64>() {
                            let bitrate_mbps = (bitrate as f64) / 1_000_000.0;

                            // REMUX detection: high bitrate
                            if (is_1080p && bitrate_mbps > 15.0) || (is_2160p && bitrate_mbps > 40.0) {
                                remux_score += 0.5;
                                reasons.push(format!(
                                    "high bitrate: {:.1} Mbps for {}x{} (REMUX indicator)",
                                    bitrate_mbps, width, height
                                ));
                            }
                            // LOW-QUALITY detection: low bitrate
                            else if is_1080p && bitrate_mbps < 5.0 {
                                lowquality_score += 0.5;
                                reasons.push(format!(
                                    "low bitrate: {:.1} Mbps for {}x{} (LOW-QUALITY indicator)",
                                    bitrate_mbps, width, height
                                ));
                            }
                            // WEB-DL range
                            else {
                                webdl_score += 0.2;
                                reasons.push(format!(
                                    "moderate bitrate: {:.1} Mbps for {}x{} (WEB-DL range)",
                                    bitrate_mbps, width, height
                                ));
                            }
                        }
                    }

                    // Bitrate per pixel analysis
                    if bpp > 0.3 {
                        remux_score += 0.2;
                        reasons.push(format!("high bitrate/pixel: {:.4} (REMUX indicator)", bpp));
                    } else if bpp < 0.1 {
                        lowquality_score += 0.2;
                        reasons.push(format!("low bitrate/pixel: {:.4} (LOW-QUALITY indicator)", bpp));
                    }
                }
            }
        }

        // === SIGNAL 2: Audio codec analysis (strong REMUX indicator) ===
        let audio_streams: Vec<_> = streams.iter()
            .filter(|s| s.codec_type.as_deref() == Some("audio"))
            .collect();

        for audio_stream in &audio_streams {
            if let Some(ref codec) = audio_stream.codec_name {
                let codec_lower = codec.to_lowercase();
                // Lossless audio codecs indicate REMUX
                if codec_lower == "truehd" || codec_lower == "dts" || 
                   codec_lower == "flac" || codec_lower.contains("pcm") {
                    remux_score += 0.4;
                    reasons.push(format!("lossless audio codec: {} (REMUX indicator)", codec));
                    break;
                }
                // Lossy codecs common in web sources
                else if codec_lower == "aac" || codec_lower == "opus" {
                    webdl_score += 0.1;
                    reasons.push(format!("lossy audio codec: {} (WEB-DL indicator)", codec));
                    break;
                }
            }
        }

        // === SIGNAL 3: Video codec analysis ===
        if let Some(vs) = video_stream {
            if let Some(ref codec) = vs.codec_name {
                let codec_lower = codec.to_lowercase();
                // Modern codecs indicate WEB-DL (already encoded)
                if codec_lower == "hevc" || codec_lower == "av1" || codec_lower == "vp9" {
                    webdl_score += 0.3;
                    reasons.push(format!("modern codec: {} (WEB-DL indicator)", codec));
                }
                // H.264 is ambiguous but common in both WEB-DL and older sources
                else if codec_lower == "h264" {
                    webdl_score += 0.1;
                    reasons.push(format!("h264 codec (common in WEB-DL)"));
                }
            }
        }

        // === SIGNAL 4: Filename analysis ===
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_uppercase();

        // REMUX indicators in filename
        let remux_tokens = ["REMUX", "BLURAY", "BDRIP", "BDMV", "UHD.BLURAY"];
        for token in &remux_tokens {
            if filename.contains(token) {
                remux_score += 0.3;
                reasons.push(format!("filename contains {} (REMUX indicator)", token));
                break;
            }
        }

        // WEB-DL indicators in filename
        let webdl_tokens = ["WEB-DL", "WEBRIP", "WEB", "NF", "AMZN", "HULU", "DSNP", "ATVP"];
        for token in &webdl_tokens {
            if filename.contains(token) {
                webdl_score += 0.3;
                reasons.push(format!("filename contains {} (WEB-DL indicator)", token));
                break;
            }
        }

        // === SIGNAL 5: Stream count analysis ===
        let audio_count = audio_streams.len();
        let subtitle_count = streams.iter()
            .filter(|s| s.codec_type.as_deref() == Some("subtitle"))
            .count();

        if audio_count >= 3 || subtitle_count >= 5 {
            remux_score += 0.2;
            reasons.push(format!(
                "many streams: {} audio, {} subs (REMUX indicator)",
                audio_count, subtitle_count
            ));
        } else if audio_count == 1 && subtitle_count <= 2 {
            webdl_score += 0.1;
            reasons.push(format!(
                "minimal streams: {} audio, {} subs (WEB-DL indicator)",
                audio_count, subtitle_count
            ));
        }

        // === Determine tier based on scores ===
        let max_score = remux_score.max(webdl_score).max(lowquality_score);
        let confidence = max_score;

        let tier = if remux_score >= webdl_score && remux_score >= lowquality_score {
            if remux_score > 0.0 {
                QualityTier::Remux
            } else {
                // Default to higher tier when uncertain
                QualityTier::WebDl
            }
        } else if webdl_score >= lowquality_score {
            QualityTier::WebDl
        } else {
            if lowquality_score > 0.0 {
                QualityTier::LowQuality
            } else {
                // Default to higher tier when uncertain
                QualityTier::WebDl
            }
        };

        // If confidence is low and we're at LowQuality, default to higher tier
        if confidence < 0.3 && matches!(tier, QualityTier::LowQuality) {
            reasons.push("low confidence, defaulting to higher tier".to_string());
            return SourceClassification {
                tier: QualityTier::WebDl,
                confidence,
                reasons,
                bitrate_per_pixel,
            };
        }

        let classification = SourceClassification {
            tier,
            confidence,
            reasons,
            bitrate_per_pixel,
        };

        // Log the classification decision
        info!("✅ Classification complete: {:?} (confidence: {:.2})", tier, confidence);
        
        classification
    }

    /// Calculate bitrate per pixel for classification
    fn calculate_bitrate_per_pixel(&self, format: &FFProbeFormat, video_stream: Option<&FFProbeStream>) -> Option<f64> {
        if let Some(ref bitrate_str) = format.bit_rate {
            if let Ok(bitrate) = bitrate_str.parse::<u64>() {
                if let Some(vs) = video_stream {
                    if let (Some(width), Some(height)) = (vs.width, vs.height) {
                        let pixels = (width * height) as f64;
                        return Some((bitrate as f64) / pixels);
                    }
                }
            }
        }
        None
    }

    /// Check if source should skip re-encoding (clean modern codecs in WEB-DL tier)
    /// 
    /// This method determines if a source should skip re-encoding based on its
    /// classification and codec. It should be used in conjunction with the
    /// `force_reencode` configuration flag:
    /// 
    /// ```ignore
    /// if !cfg.force_reencode && classifier.should_skip_encode(&classification, &streams) {
    ///     // Skip re-encoding
    /// }
    /// ```
    /// 
    /// Returns `true` if:
    /// - Source is classified as WEB-DL tier, AND
    /// - Video codec is already HEVC, AV1, or VP9
    /// 
    /// Returns `false` for:
    /// - REMUX tier (always re-encode for quality)
    /// - LOW-QUALITY tier (always re-encode for size reduction)
    /// - WEB-DL with older codecs like H.264 (should re-encode)
    pub fn should_skip_encode(&self, classification: &SourceClassification, streams: &[FFProbeStream]) -> bool {
        // Only consider skipping for WEB-DL tier
        if !matches!(classification.tier, QualityTier::WebDl) {
            return false;
        }

        // Check if video codec is already modern and efficient
        let video_stream = streams.iter()
            .find(|s| s.codec_type.as_deref() == Some("video"));

        if let Some(vs) = video_stream {
            if let Some(ref codec) = vs.codec_name {
                let codec_lower = codec.to_lowercase();
                // Skip re-encoding if already HEVC, AV1, or VP9
                if codec_lower == "hevc" || codec_lower == "av1" || codec_lower == "vp9" {
                    return true;
                }
            }
        }

        false
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



#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    // Strategy to generate valid video resolutions
    fn video_resolution() -> impl Strategy<Value = (i32, i32)> {
        prop_oneof![
            Just((1920, 1080)), // 1080p
            Just((3840, 2160)), // 4K
            Just((1280, 720)),  // 720p
            Just((2560, 1440)), // 1440p
        ]
    }

    // Strategy to generate bitrates (in bps)
    fn bitrate_bps() -> impl Strategy<Value = u64> {
        prop_oneof![
            2_000_000u64..5_000_000,   // Low quality range
            5_000_000u64..15_000_000,  // Mid range
            15_000_000u64..50_000_000, // High quality range
            50_000_000u64..100_000_000, // Very high quality
        ]
    }

    // Strategy to generate video codec names
    fn video_codec() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("h264".to_string()),
            Just("hevc".to_string()),
            Just("av1".to_string()),
            Just("vp9".to_string()),
            Just("mpeg2video".to_string()),
        ]
    }

    // Strategy to generate audio codec names
    fn audio_codec() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("aac".to_string()),
            Just("opus".to_string()),
            Just("truehd".to_string()),
            Just("dts".to_string()),
            Just("flac".to_string()),
            Just("pcm_s16le".to_string()),
        ]
    }

    // Helper to create a test FFProbeStream
    fn create_video_stream(
        width: i32,
        height: i32,
        codec: String,
    ) -> FFProbeStream {
        FFProbeStream {
            index: 0,
            codec_type: Some("video".to_string()),
            codec_name: Some(codec),
            width: Some(width),
            height: Some(height),
            avg_frame_rate: Some("24/1".to_string()),
            r_frame_rate: Some("24/1".to_string()),
            tags: None,
            bit_rate: None,
            disposition: None,
            pix_fmt: None,
            bits_per_raw_sample: None,
            color_transfer: None,
            color_primaries: None,
            color_space: None,
        }
    }

    // Helper to create a test FFProbeStream for audio
    fn create_audio_stream(codec: String) -> FFProbeStream {
        FFProbeStream {
            index: 1,
            codec_type: Some("audio".to_string()),
            codec_name: Some(codec),
            width: None,
            height: None,
            avg_frame_rate: None,
            r_frame_rate: None,
            tags: None,
            bit_rate: None,
            disposition: None,
            pix_fmt: None,
            bits_per_raw_sample: None,
            color_transfer: None,
            color_primaries: None,
            color_space: None,
        }
    }

    // Helper to create a test FFProbeFormat
    fn create_format(bitrate: u64) -> FFProbeFormat {
        FFProbeFormat {
            format_name: "matroska,webm".to_string(),
            bit_rate: Some(bitrate.to_string()),
            tags: None,
            muxing_app: None,
            writing_library: None,
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// **Feature: software-av1-encoding, Property 10: Classification produces valid tier**
        /// **Validates: Requirements 3.1**
        #[test]
        fn test_classification_produces_valid_tier(
            resolution in video_resolution(),
            bitrate in bitrate_bps(),
            video_codec in video_codec(),
            audio_codec in audio_codec(),
        ) {
            let classifier = SourceClassifier::new();
            let path = Path::new("test_video.mkv");
            
            let video_stream = create_video_stream(resolution.0, resolution.1, video_codec);
            let audio_stream = create_audio_stream(audio_codec);
            let streams = vec![video_stream, audio_stream];
            let format = create_format(bitrate);

            let classification = classifier.classify(path, &format, &streams);

            // Property: classification must produce exactly one valid tier
            prop_assert!(
                matches!(classification.tier, QualityTier::Remux | QualityTier::WebDl | QualityTier::LowQuality),
                "Classification must produce a valid QualityTier, got: {:?}",
                classification.tier
            );
        }

        /// **Feature: software-av1-encoding, Property 11: High bitrate REMUX classification**
        /// **Validates: Requirements 3.2**
        #[test]
        fn test_high_bitrate_remux_classification(
            is_1080p in prop::bool::ANY,
        ) {
            let classifier = SourceClassifier::new();
            let path = Path::new("test_remux.mkv");
            
            // Set resolution and bitrate based on test case
            let (width, height, bitrate) = if is_1080p {
                (1920, 1080, 20_000_000u64) // 20 Mbps for 1080p (> 15 Mbps threshold)
            } else {
                (3840, 2160, 50_000_000u64) // 50 Mbps for 2160p (> 40 Mbps threshold)
            };

            let video_stream = create_video_stream(width, height, "h264".to_string());
            let streams = vec![video_stream];
            let format = create_format(bitrate);

            let classification = classifier.classify(path, &format, &streams);

            // Property: high bitrate sources should be classified as REMUX
            prop_assert_eq!(
                classification.tier,
                QualityTier::Remux,
                "High bitrate source ({}x{} @ {} Mbps) should be classified as REMUX, got: {:?}",
                width, height, bitrate / 1_000_000, classification.tier
            );
        }

        /// **Feature: software-av1-encoding, Property 12: Modern codec WEB-DL classification**
        /// **Validates: Requirements 3.3**
        #[test]
        fn test_modern_codec_webdl_classification(
            modern_codec in prop_oneof![
                Just("hevc".to_string()),
                Just("av1".to_string()),
                Just("vp9".to_string()),
            ],
        ) {
            let classifier = SourceClassifier::new();
            let path = Path::new("test_webdl.mkv");
            
            // Use moderate bitrate that won't trigger REMUX classification
            let bitrate = 8_000_000u64; // 8 Mbps
            let video_stream = create_video_stream(1920, 1080, modern_codec.clone());
            let audio_stream = create_audio_stream("aac".to_string());
            let streams = vec![video_stream, audio_stream];
            let format = create_format(bitrate);

            let classification = classifier.classify(path, &format, &streams);

            // Property: modern codecs with moderate bitrate should be classified as WEB-DL
            prop_assert_eq!(
                classification.tier,
                QualityTier::WebDl,
                "Modern codec {} with moderate bitrate should be classified as WEB-DL, got: {:?}",
                modern_codec, classification.tier
            );
        }

        /// **Feature: software-av1-encoding, Property 13: Low bitrate LOW-QUALITY classification**
        /// **Validates: Requirements 3.4**
        #[test]
        fn test_low_bitrate_lowquality_classification(
            low_bitrate in 2_000_000u64..5_000_000, // 2-5 Mbps (below 5 Mbps threshold)
        ) {
            let classifier = SourceClassifier::new();
            let path = Path::new("test_lowquality.mkv");
            
            let video_stream = create_video_stream(1920, 1080, "h264".to_string());
            let audio_stream = create_audio_stream("aac".to_string());
            let streams = vec![video_stream, audio_stream];
            let format = create_format(low_bitrate);

            let classification = classifier.classify(path, &format, &streams);

            // Property: low bitrate 1080p sources should be classified as LOW-QUALITY
            prop_assert_eq!(
                classification.tier,
                QualityTier::LowQuality,
                "Low bitrate 1080p source ({} Mbps) should be classified as LOW-QUALITY, got: {:?}",
                low_bitrate / 1_000_000, classification.tier
            );
        }

        /// **Feature: software-av1-encoding, Property 14: Uncertain classification defaults to higher tier**
        /// **Validates: Requirements 3.5**
        #[test]
        fn test_uncertain_classification_defaults(
            resolution in video_resolution(),
        ) {
            let classifier = SourceClassifier::new();
            let path = Path::new("test_uncertain.mkv");
            
            // Create minimal metadata that provides little classification signal
            // Use moderate bitrate that doesn't strongly indicate any tier
            let bitrate = 10_000_000u64; // 10 Mbps - ambiguous
            let video_stream = create_video_stream(resolution.0, resolution.1, "h264".to_string());
            let streams = vec![video_stream];
            let format = create_format(bitrate);

            let classification = classifier.classify(path, &format, &streams);

            // Property: when uncertain, should default to higher tier (not LOW-QUALITY)
            // This means it should be either REMUX or WEB-DL, never LOW-QUALITY with low confidence
            if classification.confidence < 0.3 {
                prop_assert!(
                    !matches!(classification.tier, QualityTier::LowQuality),
                    "Uncertain classification (confidence: {:.2}) should not default to LOW-QUALITY, got: {:?}",
                    classification.confidence, classification.tier
                );
            }
        }

        /// **Feature: software-av1-encoding, Property 15: Skip re-encoding for clean modern codecs**
        /// **Validates: Requirements 6.1**
        #[test]
        fn test_skip_reencoding_for_clean_modern_codecs(
            modern_codec in prop_oneof![
                Just("hevc".to_string()),
                Just("av1".to_string()),
                Just("vp9".to_string()),
            ],
        ) {
            let classifier = SourceClassifier::new();
            let path = Path::new("test_webdl_modern.mkv");
            
            // Use moderate bitrate that will classify as WEB-DL
            let bitrate = 8_000_000u64; // 8 Mbps
            let video_stream = create_video_stream(1920, 1080, modern_codec.clone());
            let audio_stream = create_audio_stream("aac".to_string());
            let streams = vec![video_stream.clone(), audio_stream];
            let format = create_format(bitrate);

            let classification = classifier.classify(path, &format, &streams);
            
            // Verify it's classified as WEB-DL
            prop_assert_eq!(
                classification.tier,
                QualityTier::WebDl,
                "Modern codec {} should be classified as WEB-DL",
                modern_codec
            );

            // Property: WEB-DL sources with modern codecs (HEVC, AV1, VP9) should skip re-encoding
            let should_skip = classifier.should_skip_encode(&classification, &[video_stream]);
            
            prop_assert!(
                should_skip,
                "WEB-DL source with modern codec {} should skip re-encoding, but should_skip_encode returned false",
                modern_codec
            );
        }

        /// Test that non-modern codecs in WEB-DL tier do NOT skip re-encoding
        #[test]
        fn test_no_skip_for_h264_webdl(
            old_codec in prop_oneof![
                Just("h264".to_string()),
                Just("mpeg2video".to_string()),
                Just("mpeg4".to_string()),
            ],
        ) {
            let classifier = SourceClassifier::new();
            let path = Path::new("test_webdl_h264.mkv");
            
            // Use moderate bitrate that will classify as WEB-DL
            let bitrate = 8_000_000u64; // 8 Mbps
            let video_stream = create_video_stream(1920, 1080, old_codec.clone());
            let audio_stream = create_audio_stream("aac".to_string());
            let streams = vec![video_stream.clone(), audio_stream];
            let format = create_format(bitrate);

            let classification = classifier.classify(path, &format, &streams);

            // Property: WEB-DL sources with older codecs should NOT skip re-encoding
            let should_skip = classifier.should_skip_encode(&classification, &[video_stream]);
            
            prop_assert!(
                !should_skip,
                "WEB-DL source with older codec {} should NOT skip re-encoding, but should_skip_encode returned true",
                old_codec
            );
        }

        /// Test that REMUX tier never skips re-encoding, even with modern codecs
        #[test]
        fn test_remux_never_skips(
            codec in video_codec(),
        ) {
            let classifier = SourceClassifier::new();
            let path = Path::new("test_remux.mkv");
            
            // Use high bitrate that will classify as REMUX
            let bitrate = 25_000_000u64; // 25 Mbps for 1080p
            let video_stream = create_video_stream(1920, 1080, codec.clone());
            let audio_stream = create_audio_stream("truehd".to_string()); // Lossless audio
            let streams = vec![video_stream.clone(), audio_stream];
            let format = create_format(bitrate);

            let classification = classifier.classify(path, &format, &streams);
            
            // Verify it's classified as REMUX
            prop_assert_eq!(
                classification.tier,
                QualityTier::Remux,
                "High bitrate source should be classified as REMUX"
            );

            // Property: REMUX sources should NEVER skip re-encoding, regardless of codec
            let should_skip = classifier.should_skip_encode(&classification, &[video_stream]);
            
            prop_assert!(
                !should_skip,
                "REMUX source with codec {} should NEVER skip re-encoding, but should_skip_encode returned true",
                codec
            );
        }

        /// Test that LOW-QUALITY tier never skips re-encoding
        #[test]
        fn test_lowquality_never_skips(
            codec in video_codec(),
        ) {
            let classifier = SourceClassifier::new();
            let path = Path::new("test_lowquality.mkv");
            
            // Use very low bitrate with old codec to ensure LOW-QUALITY classification
            // Modern codecs might override low bitrate signal, so use h264/mpeg2
            let bitrate = 3_000_000u64; // 3 Mbps for 1080p
            let old_codec = if codec == "hevc" || codec == "av1" || codec == "vp9" {
                "h264".to_string()
            } else {
                codec.clone()
            };
            let video_stream = create_video_stream(1920, 1080, old_codec.clone());
            let audio_stream = create_audio_stream("aac".to_string());
            let streams = vec![video_stream.clone(), audio_stream];
            let format = create_format(bitrate);

            let classification = classifier.classify(path, &format, &streams);
            
            // Only test the skip logic if it's actually classified as LOW-QUALITY
            // (Modern codecs might override low bitrate signal per requirement 3.3)
            if matches!(classification.tier, QualityTier::LowQuality) {
                // Property: LOW-QUALITY sources should NEVER skip re-encoding, regardless of codec
                let should_skip = classifier.should_skip_encode(&classification, &[video_stream]);
                
                prop_assert!(
                    !should_skip,
                    "LOW-QUALITY source with codec {} should NEVER skip re-encoding, but should_skip_encode returned true",
                    old_codec
                );
            }
        }
    }
}
