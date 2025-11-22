use crate::classifier::{QualityTier, SourceClassification};
use crate::ffmpeg_native::AV1Encoder;
use crate::ffprobe::{FFProbeData, BitDepth};

/// Encoding parameters for software AV1 encoding
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodingParams {
    pub crf: u8,
    pub preset: u8,
    pub tune: Option<u8>,
    pub film_grain: Option<u8>,
    pub bit_depth: BitDepth,
    pub pixel_format: String,
}

/// Quality calculator for determining CRF, preset, and encoding parameters
pub struct QualityCalculator;

impl QualityCalculator {
    /// Create a new quality calculator
    pub fn new() -> Self {
        QualityCalculator
    }

    /// Calculate encoding parameters for a source with quality-first decision logging
    pub fn calculate_params(
        &self,
        classification: &SourceClassification,
        meta: &FFProbeData,
        encoder: &AV1Encoder,
    ) -> EncodingParams {
        let params = self.calculate_params_internal(classification, meta, encoder);
        
        // Log quality-first decisions
        self.log_quality_decisions(classification, &params, meta);
        
        params
    }

    /// Internal method to calculate encoding parameters without logging
    fn calculate_params_internal(
        &self,
        classification: &SourceClassification,
        meta: &FFProbeData,
        encoder: &AV1Encoder,
    ) -> EncodingParams {
        // Get video stream for analysis
        let video_stream = meta.streams.iter()
            .find(|s| s.codec_type.as_deref() == Some("video"));

        // Detect bit depth
        let bit_depth = video_stream
            .map(|s| s.detect_bit_depth())
            .unwrap_or(BitDepth::Bit8);

        // Determine pixel format based on bit depth
        let pixel_format = match bit_depth {
            BitDepth::Bit8 => "yuv420p".to_string(),
            BitDepth::Bit10 => "yuv420p10le".to_string(),
            BitDepth::Unknown => "yuv420p10le".to_string(), // Default to 10-bit to avoid quality loss
        };

        // Get resolution for CRF calculation
        let height = video_stream
            .and_then(|s| s.height)
            .unwrap_or(1080);

        // Calculate CRF based on tier and resolution
        let crf = self.calculate_crf(&classification.tier, height);

        // Calculate preset based on tier
        let preset = self.calculate_preset(&classification.tier);

        // Determine film-grain parameter (REMUX only)
        let film_grain = if matches!(classification.tier, QualityTier::Remux) {
            Some(8)
        } else {
            None
        };

        // Determine tune parameter (SVT-AV1-PSY only)
        let tune = if matches!(encoder, AV1Encoder::SvtAv1Psy) {
            Some(3)
        } else {
            None
        };

        EncodingParams {
            crf,
            preset,
            tune,
            film_grain,
            bit_depth,
            pixel_format,
        }
    }

    /// Calculate CRF value based on quality tier and resolution
    fn calculate_crf(&self, tier: &QualityTier, height: i32) -> u8 {
        match tier {
            QualityTier::Remux => {
                // REMUX: Quality-first, preserve everything
                if height >= 2160 {
                    // 4K: CRF 20
                    20
                } else {
                    // 1080p and below: CRF 18
                    18
                }
            }
            QualityTier::WebDl => {
                // WEB-DL: Conservative re-encoding
                if height >= 2160 {
                    // 4K: CRF 28
                    28
                } else {
                    // 1080p and below: CRF 26
                    26
                }
            }
            QualityTier::LowQuality => {
                // LOW-QUALITY: Size reduction OK
                30
            }
        }
    }

    /// Calculate preset value based on quality tier
    fn calculate_preset(&self, tier: &QualityTier) -> u8 {
        match tier {
            QualityTier::Remux => {
                // REMUX: Slower preset for maximum quality (preset 3)
                3
            }
            QualityTier::WebDl => {
                // WEB-DL: Balanced preset (preset 5)
                5
            }
            QualityTier::LowQuality => {
                // LOW-QUALITY: Faster preset (preset 6)
                6
            }
        }
    }

    /// Log quality-first decisions with reasoning
    fn log_quality_decisions(
        &self,
        classification: &SourceClassification,
        params: &EncodingParams,
        meta: &FFProbeData,
    ) {
        use log::info;

        let video_stream = meta.streams.iter()
            .find(|s| s.codec_type.as_deref() == Some("video"));
        
        let resolution = video_stream
            .and_then(|s| s.width.zip(s.height))
            .map(|(w, h)| format!("{}x{}", w, h))
            .unwrap_or_else(|| "unknown".to_string());

        // Log tier classification decision
        info!("📊 Quality tier classification: {:?} (confidence: {:.2})", 
              classification.tier, classification.confidence);
        
        if !classification.reasons.is_empty() {
            info!("   Reasons: {}", classification.reasons.join(", "));
        }

        // Log CRF selection decision with quality-first reasoning
        let crf_reasoning = self.get_crf_reasoning(&classification.tier, params.crf, &resolution);
        info!("🎯 CRF selection: {} - {}", params.crf, crf_reasoning);

        // Log preset selection decision with quality-first reasoning
        let preset_reasoning = self.get_preset_reasoning(&classification.tier, params.preset);
        info!("⚙️  Preset selection: {} - {}", params.preset, preset_reasoning);

        // Log film-grain decision
        if let Some(grain) = params.film_grain {
            info!("🌾 Film-grain synthesis: enabled (value: {}) - preserving natural grain texture for REMUX source", grain);
        } else {
            match classification.tier {
                QualityTier::Remux => {
                    info!("🌾 Film-grain synthesis: disabled - no visible grain detected in source");
                }
                _ => {
                    info!("🌾 Film-grain synthesis: disabled - avoiding artificial texture in already-encoded source");
                }
            }
        }

        // Log tune parameter decision
        if let Some(tune) = params.tune {
            info!("🎨 Perceptual tuning: enabled (tune={}) - SVT-AV1-PSY grain-optimized encoding for maximum quality", tune);
        }

        // Log bit depth decision
        info!("🎨 Bit depth: {:?} (pixel format: {}) - preserving source color depth", 
              params.bit_depth, params.pixel_format);

        // Log overall quality-first philosophy
        info!("✨ Quality-first encoding: prioritizing perceptual quality over file size and encoding speed");
    }

    /// Get reasoning for CRF selection
    fn get_crf_reasoning(&self, tier: &QualityTier, _crf: u8, resolution: &str) -> String {
        match tier {
            QualityTier::Remux => {
                format!(
                    "Quality-first for REMUX source at {}. Lower CRF prioritizes preserving all grain, texture, and gradients. \
                     Encoding time and file size are secondary to visual fidelity.",
                    resolution
                )
            }
            QualityTier::WebDl => {
                format!(
                    "Conservative re-encoding for WEB-DL source at {}. Avoiding compounding existing compression artifacts. \
                     Quality preservation prioritized over aggressive compression.",
                    resolution
                )
            }
            QualityTier::LowQuality => {
                format!(
                    "Size reduction for LOW-QUALITY source at {}. Source already degraded, \
                     but CRF kept reasonable to avoid excessive quality loss.",
                    resolution
                )
            }
        }
    }

    /// Get reasoning for preset selection
    fn get_preset_reasoning(&self, tier: &QualityTier, preset: u8) -> String {
        match tier {
            QualityTier::Remux => {
                format!(
                    "Slower preset (preset {}) prioritizes maximum quality for REMUX source. \
                     Longer encoding time is acceptable to achieve best compression decisions and artifact-free output.",
                    preset
                )
            }
            QualityTier::WebDl => {
                format!(
                    "Balanced preset (preset {}) for WEB-DL source. \
                     Quality-focused while avoiding excessive encoding time for already-encoded content.",
                    preset
                )
            }
            QualityTier::LowQuality => {
                format!(
                    "Faster preset (preset {}) acceptable for LOW-QUALITY source. \
                     Source already degraded, so faster encoding is reasonable.",
                    preset
                )
            }
        }
    }

    /// Get decision reasoning for testing purposes
    #[cfg(test)]
    pub fn get_decision_reasoning(
        &self,
        classification: &SourceClassification,
        params: &EncodingParams,
        height: i32,
    ) -> String {
        let resolution = format!("{}p", height);
        let crf_reasoning = self.get_crf_reasoning(&classification.tier, params.crf, &resolution);
        let preset_reasoning = self.get_preset_reasoning(&classification.tier, params.preset);
        
        format!(
            "Tier: {:?}, CRF: {} ({}), Preset: {} ({}), Film-grain: {:?}",
            classification.tier,
            params.crf,
            crf_reasoning,
            params.preset,
            preset_reasoning,
            params.film_grain
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffprobe::{FFProbeStream, FFProbeFormat};
    use proptest::prelude::*;

    // Helper to create test FFProbeData
    fn create_test_metadata(width: i32, height: i32, bit_depth: BitDepth) -> FFProbeData {
        let pix_fmt = match bit_depth {
            BitDepth::Bit8 => Some("yuv420p"),
            BitDepth::Bit10 => Some("yuv420p10le"),
            BitDepth::Unknown => None, // No pixel format info for unknown
        };

        let bits_per_raw_sample = match bit_depth {
            BitDepth::Bit8 => Some("8".to_string()),
            BitDepth::Bit10 => Some("10".to_string()),
            BitDepth::Unknown => None,
        };

        FFProbeData {
            streams: vec![FFProbeStream {
                index: 0,
                codec_type: Some("video".to_string()),
                codec_name: Some("h264".to_string()),
                width: Some(width),
                height: Some(height),
                pix_fmt: pix_fmt.map(|s| s.to_string()),
                bits_per_raw_sample,
                avg_frame_rate: Some("24/1".to_string()),
                r_frame_rate: Some("24/1".to_string()),
                bit_rate: Some("10000000".to_string()),
                tags: None,
                disposition: None,
                color_transfer: None,
                color_primaries: None,
                color_space: None,
            }],
            format: FFProbeFormat {
                format_name: "matroska,webm".to_string(),
                bit_rate: Some("10000000".to_string()),
                tags: None,
                muxing_app: None,
                writing_library: None,
            },
        }
    }

    // Helper to create test classification
    fn create_test_classification(tier: QualityTier) -> SourceClassification {
        SourceClassification {
            tier,
            confidence: 0.8,
            reasons: vec![],
            bitrate_per_pixel: None,
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// **Feature: software-av1-encoding, Property 21: CRF selection by tier and resolution**
        /// **Validates: Requirements 5.1, 5.2, 6.2, 6.3, 7.1**
        #[test]
        fn test_crf_selection_by_tier_and_resolution(
            tier in prop_oneof![
                Just(QualityTier::Remux),
                Just(QualityTier::WebDl),
                Just(QualityTier::LowQuality),
            ],
            is_4k in prop::bool::ANY,
        ) {
            let calculator = QualityCalculator::new();
            let (width, height) = if is_4k {
                (3840, 2160)
            } else {
                (1920, 1080)
            };

            let meta = create_test_metadata(width, height, BitDepth::Bit8);
            let classification = create_test_classification(tier);
            let encoder = AV1Encoder::SvtAv1;

            let params = calculator.calculate_params(&classification, &meta, &encoder);

            // Determine expected CRF based on tier and resolution
            let expected_crf = match (tier, is_4k) {
                (QualityTier::Remux, true) => 20,   // REMUX 2160p: CRF 20
                (QualityTier::Remux, false) => 18,  // REMUX 1080p: CRF 18
                (QualityTier::WebDl, true) => 28,   // WEB-DL 2160p: CRF 28
                (QualityTier::WebDl, false) => 26,  // WEB-DL 1080p: CRF 26
                (QualityTier::LowQuality, _) => 30, // LOW-QUALITY: CRF 30
            };

            prop_assert_eq!(
                params.crf,
                expected_crf,
                "CRF for {:?} at {}x{} should be {}, got {}",
                tier, width, height, expected_crf, params.crf
            );
        }

        /// **Feature: software-av1-encoding, Property 22: Preset selection by tier**
        /// **Validates: Requirements 5.3, 6.4, 7.2**
        #[test]
        fn test_preset_selection_by_tier(
            tier in prop_oneof![
                Just(QualityTier::Remux),
                Just(QualityTier::WebDl),
                Just(QualityTier::LowQuality),
            ],
            width in 1280i32..3840,
            height in 720i32..2160,
        ) {
            let calculator = QualityCalculator::new();
            let meta = create_test_metadata(width, height, BitDepth::Bit8);
            let classification = create_test_classification(tier);
            let encoder = AV1Encoder::SvtAv1;

            let params = calculator.calculate_params(&classification, &meta, &encoder);

            // Determine expected preset based on tier
            let expected_preset = match tier {
                QualityTier::Remux => 3,      // REMUX: preset 3 (slower)
                QualityTier::WebDl => 5,      // WEB-DL: preset 5 (balanced)
                QualityTier::LowQuality => 6, // LOW-QUALITY: preset 6 (faster)
            };

            prop_assert_eq!(
                params.preset,
                expected_preset,
                "Preset for {:?} should be {}, got {}",
                tier, expected_preset, params.preset
            );

            // Verify preset constraints from requirements
            match tier {
                QualityTier::Remux => {
                    prop_assert!(
                        params.preset <= 3,
                        "REMUX preset should be ≤3, got {}",
                        params.preset
                    );
                }
                QualityTier::WebDl => {
                    prop_assert_eq!(
                        params.preset,
                        5,
                        "WEB-DL preset should be 5, got {}",
                        params.preset
                    );
                }
                QualityTier::LowQuality => {
                    prop_assert!(
                        params.preset >= 6,
                        "LOW-QUALITY preset should be ≥6, got {}",
                        params.preset
                    );
                }
            }
        }

        /// **Feature: software-av1-encoding, Property 23: Film-grain enabled for REMUX only**
        /// **Validates: Requirements 6.5, 7.3**
        #[test]
        fn test_film_grain_enabled_for_remux_only(
            tier in prop_oneof![
                Just(QualityTier::Remux),
                Just(QualityTier::WebDl),
                Just(QualityTier::LowQuality),
            ],
            width in 1280i32..3840,
            height in 720i32..2160,
        ) {
            let calculator = QualityCalculator::new();
            let meta = create_test_metadata(width, height, BitDepth::Bit8);
            let classification = create_test_classification(tier);
            let encoder = AV1Encoder::SvtAv1;

            let params = calculator.calculate_params(&classification, &meta, &encoder);

            // Film-grain should be enabled (value 8) for REMUX only
            match tier {
                QualityTier::Remux => {
                    prop_assert_eq!(
                        params.film_grain,
                        Some(8),
                        "REMUX sources should have film-grain enabled (value 8), got {:?}",
                        params.film_grain
                    );
                }
                QualityTier::WebDl | QualityTier::LowQuality => {
                    prop_assert_eq!(
                        params.film_grain,
                        None,
                        "{:?} sources should have film-grain disabled (None), got {:?}",
                        tier, params.film_grain
                    );
                }
            }
        }

        /// **Feature: software-av1-encoding, Property 9: SVT-AV1-PSY perceptual tuning**
        /// **Validates: Requirements 2.5, 5.5**
        #[test]
        fn test_svt_av1_psy_perceptual_tuning(
            encoder in prop_oneof![
                Just(AV1Encoder::SvtAv1Psy),
                Just(AV1Encoder::SvtAv1),
                Just(AV1Encoder::LibAom),
                Just(AV1Encoder::LibRav1e),
            ],
            tier in prop_oneof![
                Just(QualityTier::Remux),
                Just(QualityTier::WebDl),
                Just(QualityTier::LowQuality),
            ],
        ) {
            let calculator = QualityCalculator::new();
            let meta = create_test_metadata(1920, 1080, BitDepth::Bit8);
            let classification = create_test_classification(tier);

            let params = calculator.calculate_params(&classification, &meta, &encoder);

            // Tune parameter should be set to 3 for SVT-AV1-PSY only
            match encoder {
                AV1Encoder::SvtAv1Psy => {
                    prop_assert_eq!(
                        params.tune,
                        Some(3),
                        "SVT-AV1-PSY should have tune=3 enabled, got {:?}",
                        params.tune
                    );
                }
                _ => {
                    prop_assert_eq!(
                        params.tune,
                        None,
                        "Non-PSY encoders should not have tune parameter, got {:?}",
                        params.tune
                    );
                }
            }
        }

        /// **Feature: software-av1-encoding, Property 26: 10-bit source produces 10-bit output**
        /// **Validates: Requirements 8.1**
        #[test]
        fn test_10bit_source_produces_10bit_output(
            tier in prop_oneof![
                Just(QualityTier::Remux),
                Just(QualityTier::WebDl),
                Just(QualityTier::LowQuality),
            ],
            width in 1280i32..3840,
            height in 720i32..2160,
        ) {
            let calculator = QualityCalculator::new();
            let meta = create_test_metadata(width, height, BitDepth::Bit10);
            let classification = create_test_classification(tier);
            let encoder = AV1Encoder::SvtAv1;

            let params = calculator.calculate_params(&classification, &meta, &encoder);

            // 10-bit source should produce 10-bit output
            prop_assert_eq!(
                params.bit_depth,
                BitDepth::Bit10,
                "10-bit source should produce 10-bit output, got {:?}",
                params.bit_depth
            );

            prop_assert_eq!(
                &params.pixel_format,
                "yuv420p10le",
                "10-bit source should use yuv420p10le pixel format, got {}",
                &params.pixel_format
            );
        }

        /// **Feature: software-av1-encoding, Property 28: 10-bit filter chain uses correct pixel format**
        /// **Validates: Requirements 8.3**
        #[test]
        fn test_10bit_filter_chain_pixel_format(
            tier in prop_oneof![
                Just(QualityTier::Remux),
                Just(QualityTier::WebDl),
                Just(QualityTier::LowQuality),
            ],
            width in 1280i32..3840,
            height in 720i32..2160,
        ) {
            let calculator = QualityCalculator::new();
            let meta = create_test_metadata(width, height, BitDepth::Bit10);
            let classification = create_test_classification(tier);
            let encoder = AV1Encoder::SvtAv1;

            let params = calculator.calculate_params(&classification, &meta, &encoder);

            // 10-bit processing should use yuv420p10le or p010le pixel format
            prop_assert!(
                params.pixel_format == "yuv420p10le" || params.pixel_format == "p010le",
                "10-bit filter chain should use yuv420p10le or p010le, got {}",
                &params.pixel_format
            );
        }

        /// **Feature: software-av1-encoding, Property 29: 8-bit source produces 8-bit output**
        /// **Validates: Requirements 8.4**
        #[test]
        fn test_8bit_source_produces_8bit_output(
            tier in prop_oneof![
                Just(QualityTier::Remux),
                Just(QualityTier::WebDl),
                Just(QualityTier::LowQuality),
            ],
            width in 1280i32..3840,
            height in 720i32..2160,
        ) {
            let calculator = QualityCalculator::new();
            let meta = create_test_metadata(width, height, BitDepth::Bit8);
            let classification = create_test_classification(tier);
            let encoder = AV1Encoder::SvtAv1;

            let params = calculator.calculate_params(&classification, &meta, &encoder);

            // 8-bit source should produce 8-bit output without upconverting
            prop_assert_eq!(
                params.bit_depth,
                BitDepth::Bit8,
                "8-bit source should produce 8-bit output, got {:?}",
                params.bit_depth
            );

            prop_assert_eq!(
                &params.pixel_format,
                "yuv420p",
                "8-bit source should use yuv420p pixel format, got {}",
                &params.pixel_format
            );
        }

        /// **Feature: software-av1-encoding, Property 30: Unknown bit depth defaults to 10-bit**
        /// **Validates: Requirements 8.5**
        #[test]
        fn test_unknown_bit_depth_defaults_to_10bit(
            tier in prop_oneof![
                Just(QualityTier::Remux),
                Just(QualityTier::WebDl),
                Just(QualityTier::LowQuality),
            ],
            width in 1280i32..3840,
            height in 720i32..2160,
        ) {
            let calculator = QualityCalculator::new();
            let meta = create_test_metadata(width, height, BitDepth::Unknown);
            let classification = create_test_classification(tier);
            let encoder = AV1Encoder::SvtAv1;

            let params = calculator.calculate_params(&classification, &meta, &encoder);

            // Unknown bit depth should default to 10-bit to avoid quality loss
            prop_assert_eq!(
                &params.pixel_format,
                "yuv420p10le",
                "Unknown bit depth should default to yuv420p10le pixel format, got {}",
                &params.pixel_format
            );
        }

        /// **Feature: software-av1-encoding, Property 24: Quality prioritization in CRF selection**
        /// **Validates: Requirements 12.2, 12.4**
        #[test]
        fn test_quality_prioritization_in_crf_selection(
            tier in prop_oneof![
                Just(QualityTier::Remux),
                Just(QualityTier::WebDl),
                Just(QualityTier::LowQuality),
            ],
            is_4k in prop::bool::ANY,
        ) {
            let calculator = QualityCalculator::new();
            let (width, height) = if is_4k {
                (3840, 2160)
            } else {
                (1920, 1080)
            };

            let meta = create_test_metadata(width, height, BitDepth::Bit8);
            let classification = create_test_classification(tier);
            let encoder = AV1Encoder::SvtAv1;

            let params = calculator.calculate_params(&classification, &meta, &encoder);

            // Property: When choosing CRF values, system SHALL prefer lower CRF (higher quality)
            // This means CRF values should be at the quality-first end of acceptable ranges
            match (tier, is_4k) {
                (QualityTier::Remux, true) => {
                    // REMUX 2160p: CRF 20 (quality-first, not higher)
                    prop_assert!(
                        params.crf <= 20,
                        "REMUX 2160p CRF should prioritize quality (≤20), got {}",
                        params.crf
                    );
                }
                (QualityTier::Remux, false) => {
                    // REMUX 1080p: CRF 18 (quality-first, not higher)
                    prop_assert!(
                        params.crf <= 18,
                        "REMUX 1080p CRF should prioritize quality (≤18), got {}",
                        params.crf
                    );
                }
                (QualityTier::WebDl, true) => {
                    // WEB-DL 2160p: CRF 28 (conservative, not higher)
                    prop_assert!(
                        params.crf <= 28,
                        "WEB-DL 2160p CRF should prioritize quality (≤28), got {}",
                        params.crf
                    );
                }
                (QualityTier::WebDl, false) => {
                    // WEB-DL 1080p: CRF 26 (conservative, not higher)
                    prop_assert!(
                        params.crf <= 26,
                        "WEB-DL 1080p CRF should prioritize quality (≤26), got {}",
                        params.crf
                    );
                }
                (QualityTier::LowQuality, _) => {
                    // LOW-QUALITY: CRF 30 (size reduction OK, but not excessive)
                    prop_assert!(
                        params.crf <= 30,
                        "LOW-QUALITY CRF should not exceed 30, got {}",
                        params.crf
                    );
                }
            }

            // Property: CRF should never be adjusted upward to reduce file size
            // (unless user explicitly requests size optimization, which is not tested here)
            // This is validated by checking that CRF values match the quality-first defaults
            let expected_crf = match (tier, is_4k) {
                (QualityTier::Remux, true) => 20,
                (QualityTier::Remux, false) => 18,
                (QualityTier::WebDl, true) => 28,
                (QualityTier::WebDl, false) => 26,
                (QualityTier::LowQuality, _) => 30,
            };

            prop_assert_eq!(
                params.crf,
                expected_crf,
                "CRF should match quality-first default for {:?} at {}x{}, expected {}, got {}",
                tier, width, height, expected_crf, params.crf
            );
        }

        /// **Feature: software-av1-encoding, Property 25: Quality prioritization in preset selection**
        /// **Validates: Requirements 12.3, 12.4**
        #[test]
        fn test_quality_prioritization_in_preset_selection(
            tier in prop_oneof![
                Just(QualityTier::Remux),
                Just(QualityTier::WebDl),
                Just(QualityTier::LowQuality),
            ],
            width in 1280i32..3840,
            height in 720i32..2160,
        ) {
            let calculator = QualityCalculator::new();
            let meta = create_test_metadata(width, height, BitDepth::Bit8);
            let classification = create_test_classification(tier);
            let encoder = AV1Encoder::SvtAv1;

            let params = calculator.calculate_params(&classification, &meta, &encoder);

            // Property: When choosing presets, system SHALL prefer slower presets (higher quality)
            // even if encoding time increases
            match tier {
                QualityTier::Remux => {
                    // REMUX: preset 3 or slower (quality-first)
                    prop_assert!(
                        params.preset <= 3,
                        "REMUX preset should prioritize quality (≤3 for slower encoding), got {}",
                        params.preset
                    );
                }
                QualityTier::WebDl => {
                    // WEB-DL: preset 5 (balanced, but quality-focused)
                    prop_assert!(
                        params.preset <= 5,
                        "WEB-DL preset should prioritize quality (≤5), got {}",
                        params.preset
                    );
                }
                QualityTier::LowQuality => {
                    // LOW-QUALITY: preset 6 or faster (speed OK for degraded content)
                    prop_assert!(
                        params.preset >= 6,
                        "LOW-QUALITY preset should be ≥6, got {}",
                        params.preset
                    );
                }
            }

            // Property: Preset should never be increased (made faster) to save encoding time
            // (unless user explicitly requests speed optimization, which is not tested here)
            // This is validated by checking that preset values match the quality-first defaults
            let expected_preset = match tier {
                QualityTier::Remux => 3,      // Slower preset for maximum quality
                QualityTier::WebDl => 5,      // Balanced preset
                QualityTier::LowQuality => 6, // Faster preset (acceptable for degraded content)
            };

            prop_assert_eq!(
                params.preset,
                expected_preset,
                "Preset should match quality-first default for {:?}, expected {}, got {}",
                tier, expected_preset, params.preset
            );

            // Property: For REMUX tier, slower presets are preferred over faster ones
            // This means preset should be at the slow end (low numbers = slower/better)
            if matches!(tier, QualityTier::Remux) {
                prop_assert!(
                    params.preset <= 4,
                    "REMUX preset should be slow (≤4) to maximize quality, got {}",
                    params.preset
                );
            }
        }

        /// **Feature: software-av1-encoding, Property 33: Quality decision logging**
        /// **Validates: Requirements 12.5**
        #[test]
        fn test_quality_decision_logging(
            tier in prop_oneof![
                Just(QualityTier::Remux),
                Just(QualityTier::WebDl),
                Just(QualityTier::LowQuality),
            ],
            is_4k in prop::bool::ANY,
        ) {
            let calculator = QualityCalculator::new();
            let (width, height) = if is_4k {
                (3840, 2160)
            } else {
                (1920, 1080)
            };

            let meta = create_test_metadata(width, height, BitDepth::Bit8);
            let classification = create_test_classification(tier);
            let encoder = AV1Encoder::SvtAv1;

            let params = calculator.calculate_params(&classification, &meta, &encoder);

            // Property: Quality decisions should be logged with reasoning
            // We verify this by checking that the calculate_params_with_logging method
            // returns decision reasons that explain why quality was prioritized

            let decision_log = calculator.get_decision_reasoning(&classification, &params, height);

            // Property: Decision log should contain information about CRF selection
            prop_assert!(
                decision_log.contains("CRF") || decision_log.contains("crf"),
                "Decision log should mention CRF selection, got: {}",
                decision_log
            );

            // Property: Decision log should contain information about preset selection
            prop_assert!(
                decision_log.contains("preset") || decision_log.contains("Preset"),
                "Decision log should mention preset selection, got: {}",
                decision_log
            );

            // Property: Decision log should contain information about quality tier
            prop_assert!(
                decision_log.contains("REMUX") || decision_log.contains("WEB-DL") || 
                decision_log.contains("LOW-QUALITY") || decision_log.contains("quality"),
                "Decision log should mention quality tier, got: {}",
                decision_log
            );

            // Property: Decision log should explain quality prioritization
            prop_assert!(
                decision_log.contains("quality") || decision_log.contains("preserve") || 
                decision_log.contains("prioritize") || decision_log.contains("maximum"),
                "Decision log should explain quality prioritization, got: {}",
                decision_log
            );

            // Property: For REMUX tier, log should mention grain/detail preservation
            if matches!(tier, QualityTier::Remux) {
                prop_assert!(
                    decision_log.contains("grain") || decision_log.contains("detail") || 
                    decision_log.contains("preserve") || decision_log.contains("maximum quality"),
                    "REMUX decision log should mention grain/detail preservation, got: {}",
                    decision_log
                );
            }

            // Property: Decision log should not be empty
            prop_assert!(
                !decision_log.is_empty(),
                "Decision log should not be empty"
            );

            // Property: Decision log should be informative (reasonable length)
            prop_assert!(
                decision_log.len() >= 20,
                "Decision log should be informative (at least 20 characters), got {} chars: {}",
                decision_log.len(),
                decision_log
            );
        }
    }
}
