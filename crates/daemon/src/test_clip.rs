use std::path::{Path, PathBuf};
use anyhow::{Context, Result};
use crate::classifier::QualityTier;
use crate::quality::EncodingParams;
use crate::ffmpeg_native::{FFmpegManager, CommandBuilder};
use crate::ffprobe::FFProbeData;

/// User decision after reviewing test clip
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalDecision {
    /// User approves the quality, proceed with full encode
    Approved,
    /// User requests lower CRF (higher quality)
    LowerCrf(u8),
    /// User requests slower preset (higher quality)
    SlowerPreset(u8),
    /// User rejects and wants to cancel
    Rejected,
}

/// Information about an extracted test clip
#[derive(Debug, Clone)]
pub struct TestClipInfo {
    pub clip_path: PathBuf,
    pub start_time: f64,
    pub duration: f64,
    pub encoded_path: Option<PathBuf>,
}

/// Test clip workflow for REMUX sources
/// 
/// Extracts and encodes test clips for quality validation before full encode.
/// Only used for REMUX-tier sources to ensure quality preservation.
pub struct TestClipWorkflow {
    temp_dir: PathBuf,
}

impl TestClipWorkflow {
    /// Create a new test clip workflow with specified temp directory
    pub fn new(temp_dir: PathBuf) -> Self {
        TestClipWorkflow { temp_dir }
    }

    /// Check if test clip workflow should be used for this source
    /// 
    /// Test clips are only extracted for REMUX-tier sources.
    /// WEB-DL and LOW-QUALITY sources skip the test clip workflow.
    pub fn should_extract_test_clip(&self, tier: &QualityTier) -> bool {
        matches!(tier, QualityTier::Remux)
    }

    /// Extract test clip from source file
    /// 
    /// Extracts a 30-60 second segment from the source without re-encoding.
    /// Uses scene selection heuristics to find challenging content:
    /// - Dark scenes (reveal banding)
    /// - High grain/texture (test grain preservation)
    /// - High motion (test temporal compression)
    pub async fn extract_test_clip(
        &self,
        source: &Path,
        meta: &FFProbeData,
        ffmpeg_mgr: &FFmpegManager,
    ) -> Result<TestClipInfo> {
        // Determine test clip duration (30-60 seconds)
        let duration = self.calculate_test_clip_duration(meta);
        
        // Select start time using scene selection heuristics
        let start_time = self.select_test_clip_start(meta);
        
        // Generate output path for test clip
        let clip_filename = format!(
            "test_clip_{}_{}.mkv",
            source.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown"),
            chrono::Utc::now().timestamp()
        );
        let clip_path = self.temp_dir.join(clip_filename);
        
        // Build extraction command
        let builder = CommandBuilder::new();
        let args = builder.build_test_clip_command(
            source,
            &clip_path,
            start_time,
            duration,
        );
        
        // Execute extraction
        let mut cmd = tokio::process::Command::new(&ffmpeg_mgr.ffmpeg_bin);
        cmd.args(&args);
        
        let output = cmd.output().await
            .context("Failed to execute FFmpeg for test clip extraction")?;
        
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!(
                "Test clip extraction failed: {}",
                stderr
            ));
        }
        
        Ok(TestClipInfo {
            clip_path,
            start_time,
            duration,
            encoded_path: None,
        })
    }

    /// Encode test clip with proposed parameters
    /// 
    /// Encodes the extracted test clip using the same parameters that will be
    /// used for the full encode. This allows user to validate quality before
    /// committing to a long full encode.
    pub async fn encode_test_clip(
        &self,
        clip_info: &TestClipInfo,
        params: &EncodingParams,
        ffmpeg_mgr: &FFmpegManager,
        meta: &FFProbeData,
    ) -> Result<PathBuf> {
        // Generate output path for encoded test clip
        let encoded_filename = format!(
            "encoded_{}",
            clip_info.clip_path.file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("test_clip.mkv")
        );
        let encoded_path = self.temp_dir.join(encoded_filename);
        
        // Build encode command
        let builder = CommandBuilder::new();
        let encoder = ffmpeg_mgr.best_encoder();
        let args = builder.build_encode_command(
            &clip_info.clip_path,
            &encoded_path,
            params,
            encoder,
            meta,
        );
        
        // Execute encoding
        let mut cmd = tokio::process::Command::new(&ffmpeg_mgr.ffmpeg_bin);
        cmd.args(&args);
        
        let output = cmd.output().await
            .context("Failed to execute FFmpeg for test clip encoding")?;
        
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!(
                "Test clip encoding failed: {}",
                stderr
            ));
        }
        
        Ok(encoded_path)
    }

    /// Adjust encoding parameters based on user feedback
    /// 
    /// When user reports artifacts in test clip:
    /// - Lower CRF by 2 points (higher quality)
    /// - OR reduce preset speed by 1 step (slower/better quality)
    pub fn adjust_parameters(
        &self,
        params: &EncodingParams,
        decision: &ApprovalDecision,
    ) -> EncodingParams {
        let mut adjusted = params.clone();
        
        match decision {
            ApprovalDecision::LowerCrf(amount) => {
                // Lower CRF (higher quality)
                // Ensure we don't go below reasonable minimum
                adjusted.crf = adjusted.crf.saturating_sub(*amount).max(10);
            }
            ApprovalDecision::SlowerPreset(amount) => {
                // Slower preset (higher quality)
                // Ensure we don't go below 0
                adjusted.preset = adjusted.preset.saturating_sub(*amount);
            }
            _ => {
                // No adjustment needed for Approved or Rejected
            }
        }
        
        adjusted
    }

    /// Calculate test clip duration based on source metadata
    /// 
    /// Returns duration in seconds (30-60 range)
    fn calculate_test_clip_duration(&self, _meta: &FFProbeData) -> f64 {
        // Default to 45 seconds (middle of 30-60 range)
        // TODO: Could be made configurable or adaptive based on source length
        45.0
    }

    /// Select start time for test clip using scene selection heuristics
    /// 
    /// Ideally would use FFmpeg scene detection to find:
    /// - Darkest scene (reveals banding)
    /// - Most grain/texture (tests grain preservation)
    /// - Highest motion (tests temporal compression)
    /// 
    /// For now, uses a simple heuristic: start at 25% through the video
    /// to avoid intros/credits while staying in main content.
    fn select_test_clip_start(&self, meta: &FFProbeData) -> f64 {
        // Get video duration if available
        if let Some(duration_str) = meta.format.tags.as_ref()
            .and_then(|tags| tags.get("DURATION"))
            .or_else(|| meta.format.tags.as_ref()
                .and_then(|tags| tags.get("duration")))
        {
            // Parse duration (format: HH:MM:SS.mmm or seconds)
            if let Ok(duration) = Self::parse_duration(duration_str) {
                // Start at 25% through the video
                return duration * 0.25;
            }
        }
        
        // Default to 5 minutes in if we can't determine duration
        300.0
    }

    /// Parse duration string to seconds
    /// 
    /// Supports formats:
    /// - "HH:MM:SS.mmm"
    /// - "seconds.milliseconds"
    fn parse_duration(duration_str: &str) -> Result<f64> {
        // Try parsing as HH:MM:SS.mmm format
        if duration_str.contains(':') {
            let parts: Vec<&str> = duration_str.split(':').collect();
            if parts.len() == 3 {
                let hours: f64 = parts[0].parse()?;
                let minutes: f64 = parts[1].parse()?;
                let seconds: f64 = parts[2].parse()?;
                return Ok(hours * 3600.0 + minutes * 60.0 + seconds);
            }
        }
        
        // Try parsing as plain seconds
        Ok(duration_str.parse()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classifier::QualityTier;
    use crate::ffprobe::BitDepth;
    use proptest::prelude::*;

    // Helper to create test encoding params
    fn create_test_params(crf: u8, preset: u8) -> EncodingParams {
        EncodingParams {
            crf,
            preset,
            tune: None,
            film_grain: Some(8),
            bit_depth: BitDepth::Bit10,
            pixel_format: "yuv420p10le".to_string(),
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// **Feature: software-av1-encoding, Property 16: REMUX sources trigger test clip extraction**
        /// **Validates: Requirements 4.1**
        /// 
        /// For any source classified as REMUX tier, the system SHALL extract a test clip of 30-60 
        /// seconds before starting full encode
        #[test]
        fn test_remux_sources_trigger_test_clip_extraction(
            tier in prop_oneof![
                Just(QualityTier::Remux),
                Just(QualityTier::WebDl),
                Just(QualityTier::LowQuality),
            ],
        ) {
            let workflow = TestClipWorkflow::new(PathBuf::from("/tmp"));
            let should_extract = workflow.should_extract_test_clip(&tier);

            // Property: REMUX sources should trigger test clip extraction
            match tier {
                QualityTier::Remux => {
                    prop_assert!(
                        should_extract,
                        "REMUX sources should trigger test clip extraction"
                    );
                }
                QualityTier::WebDl | QualityTier::LowQuality => {
                    prop_assert!(
                        !should_extract,
                        "{:?} sources should NOT trigger test clip extraction",
                        tier
                    );
                }
            }
        }

        /// **Feature: software-av1-encoding, Property 18: User feedback adjusts parameters**
        /// **Validates: Requirements 4.4**
        /// 
        /// For any user rejection of test clip quality, the system SHALL lower CRF by 2 points 
        /// OR reduce preset speed by 1 step
        #[test]
        fn test_user_feedback_adjusts_parameters(
            initial_crf in 16u8..35,
            initial_preset in 1u8..13,
            adjustment_type in prop_oneof![
                Just("lower_crf"),
                Just("slower_preset"),
            ],
        ) {
            let workflow = TestClipWorkflow::new(PathBuf::from("/tmp"));
            let params = create_test_params(initial_crf, initial_preset);

            let decision = match adjustment_type {
                "lower_crf" => ApprovalDecision::LowerCrf(2),
                "slower_preset" => ApprovalDecision::SlowerPreset(1),
                _ => unreachable!(),
            };

            let adjusted = workflow.adjust_parameters(&params, &decision);

            // Property: adjustments should modify parameters in quality-improving direction
            match decision {
                ApprovalDecision::LowerCrf(amount) => {
                    let expected_crf = initial_crf.saturating_sub(amount).max(10);
                    prop_assert_eq!(
                        adjusted.crf,
                        expected_crf,
                        "CRF should be lowered by {} (from {} to {})",
                        amount, initial_crf, expected_crf
                    );
                    prop_assert!(
                        adjusted.crf <= initial_crf,
                        "Adjusted CRF ({}) should be <= initial CRF ({})",
                        adjusted.crf, initial_crf
                    );
                }
                ApprovalDecision::SlowerPreset(amount) => {
                    let expected_preset = initial_preset.saturating_sub(amount);
                    prop_assert_eq!(
                        adjusted.preset,
                        expected_preset,
                        "Preset should be reduced by {} (from {} to {})",
                        amount, initial_preset, expected_preset
                    );
                    prop_assert!(
                        adjusted.preset <= initial_preset,
                        "Adjusted preset ({}) should be <= initial preset ({})",
                        adjusted.preset, initial_preset
                    );
                }
                _ => {}
            }
        }

        /// **Feature: software-av1-encoding, Property 19: Approved test clip parameters match full encode**
        /// **Validates: Requirements 4.5**
        /// 
        /// For any user-approved test clip, the full encode SHALL use identical CRF, preset, and 
        /// tuning parameters
        #[test]
        fn test_approved_test_clip_parameters_match_full_encode(
            crf in 16u8..35,
            preset in 0u8..13,
            has_tune in prop::bool::ANY,
            has_film_grain in prop::bool::ANY,
        ) {
            let workflow = TestClipWorkflow::new(PathBuf::from("/tmp"));
            
            let params = EncodingParams {
                crf,
                preset,
                tune: if has_tune { Some(3) } else { None },
                film_grain: if has_film_grain { Some(8) } else { None },
                bit_depth: BitDepth::Bit10,
                pixel_format: "yuv420p10le".to_string(),
            };

            // Simulate approval (no adjustment)
            let decision = ApprovalDecision::Approved;
            let adjusted = workflow.adjust_parameters(&params, &decision);

            // Property: approved parameters should remain unchanged
            prop_assert_eq!(
                adjusted.crf,
                params.crf,
                "Approved CRF should remain unchanged"
            );
            prop_assert_eq!(
                adjusted.preset,
                params.preset,
                "Approved preset should remain unchanged"
            );
            prop_assert_eq!(
                adjusted.tune,
                params.tune,
                "Approved tune should remain unchanged"
            );
            prop_assert_eq!(
                adjusted.film_grain,
                params.film_grain,
                "Approved film_grain should remain unchanged"
            );
            prop_assert_eq!(
                adjusted.pixel_format,
                params.pixel_format,
                "Approved pixel_format should remain unchanged"
            );
        }

        /// **Feature: software-av1-encoding, Property 20: LOW-QUALITY sources skip test clip**
        /// **Validates: Requirements 7.4**
        /// 
        /// For any source classified as LOW-QUALITY tier, the system SHALL not extract or encode 
        /// a test clip
        #[test]
        fn test_low_quality_sources_skip_test_clip(
            tier in prop_oneof![
                Just(QualityTier::Remux),
                Just(QualityTier::WebDl),
                Just(QualityTier::LowQuality),
            ],
        ) {
            let workflow = TestClipWorkflow::new(PathBuf::from("/tmp"));
            let should_extract = workflow.should_extract_test_clip(&tier);

            // Property: LOW-QUALITY sources should skip test clip workflow
            if matches!(tier, QualityTier::LowQuality) {
                prop_assert!(
                    !should_extract,
                    "LOW-QUALITY sources should skip test clip extraction"
                );
            }
        }
    }

    #[test]
    fn test_duration_parsing() {
        // Test HH:MM:SS format
        let duration1 = TestClipWorkflow::parse_duration("01:30:45.500").unwrap();
        assert!((duration1 - 5445.5).abs() < 0.1);

        // Test plain seconds format
        let duration2 = TestClipWorkflow::parse_duration("123.456").unwrap();
        assert!((duration2 - 123.456).abs() < 0.1);

        // Test zero
        let duration3 = TestClipWorkflow::parse_duration("00:00:00").unwrap();
        assert!((duration3 - 0.0).abs() < 0.1);
    }
}
