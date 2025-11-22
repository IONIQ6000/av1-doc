# Implementation Plan

- [x] 1. Set up FFmpeg manager and version validation
  - Create new `ffmpeg_native.rs` module with FFmpegManager struct
  - Implement FFmpeg version detection and parsing (≥8.0 requirement)
  - Implement encoder detection (libsvtav1, libaom-av1, librav1e, SVT-AV1-PSY)
  - Implement encoder priority selection logic
  - Add FFMPEG_BIN configuration to TranscodeConfig
  - _Requirements: 1.1, 1.2, 1.3, 1.4, 2.1, 2.2, 2.3, 2.4_

- [x] 1.1 Write property test for FFmpeg version validation
  - **Property 1: FFmpeg version validation**
  - **Validates: Requirements 1.2, 1.3**

- [x] 1.2 Write property test for FFmpeg binary path configuration
  - **Property 2: FFmpeg binary path configuration**
  - **Validates: Requirements 1.4**

- [x] 1.3 Write property test for encoder priority selection
  - **Property 6: Encoder priority selection**
  - **Validates: Requirements 2.2**

- [x] 1.4 Write property test for missing encoder error
  - **Property 7: Missing encoder error**
  - **Validates: Requirements 2.3**

- [x] 2. Implement enhanced source classification
  - Create QualityTier enum (Remux, WebDl, LowQuality)
  - Implement bitrate-per-pixel calculation for classification
  - Implement REMUX detection (high bitrate, lossless audio)
  - Implement WEB-DL detection (modern codecs, filename markers)
  - Implement LOW-QUALITY detection (low bitrate, artifacts)
  - Add uncertain classification default to higher tier
  - _Requirements: 3.1, 3.2, 3.3, 3.4, 3.5_

- [x] 2.1 Write property test for classification produces valid tier
  - **Property 10: Classification produces valid tier**
  - **Validates: Requirements 3.1**

- [x] 2.2 Write property test for high bitrate REMUX classification
  - **Property 11: High bitrate REMUX classification**
  - **Validates: Requirements 3.2**

- [x] 2.3 Write property test for modern codec WEB-DL classification
  - **Property 12: Modern codec WEB-DL classification**
  - **Validates: Requirements 3.3**

- [x] 2.4 Write property test for low bitrate LOW-QUALITY classification
  - **Property 13: Low bitrate LOW-QUALITY classification**
  - **Validates: Requirements 3.4**

- [x] 2.5 Write property test for uncertain classification defaults
  - **Property 14: Uncertain classification defaults to higher tier**
  - **Validates: Requirements 3.5**

- [x] 3. Implement quality calculator for CRF and preset selection
  - Create EncodingParams struct with CRF, preset, tune, film-grain fields
  - Implement CRF selection by tier and resolution
  - Implement preset selection by tier
  - Implement film-grain parameter logic (REMUX only)
  - Implement SVT-AV1-PSY tune=3 parameter logic
  - _Requirements: 5.1, 5.2, 5.3, 5.4, 5.5, 6.2, 6.3, 6.4, 6.5, 7.1, 7.2, 7.3_

- [x] 3.1 Write property test for CRF selection by tier and resolution
  - **Property 21: CRF selection by tier and resolution**
  - **Validates: Requirements 5.1, 5.2, 6.2, 6.3, 7.1**

- [x] 3.2 Write property test for preset selection by tier
  - **Property 22: Preset selection by tier**
  - **Validates: Requirements 5.3, 6.4, 7.2**

- [x] 3.3 Write property test for film-grain enabled for REMUX only
  - **Property 23: Film-grain enabled for REMUX only**
  - **Validates: Requirements 6.5, 7.3**

- [x] 3.4 Write property test for SVT-AV1-PSY perceptual tuning
  - **Property 9: SVT-AV1-PSY perceptual tuning**
  - **Validates: Requirements 2.5, 5.5**

- [x] 4. Implement bit depth detection and preservation
  - Enhance bit depth detection in FFProbeStream
  - Implement 10-bit output path (yuv420p10le pixel format)
  - Implement 8-bit output path (yuv420p pixel format)
  - Implement unknown bit depth default to 10-bit
  - Implement HDR metadata preservation logic
  - _Requirements: 8.1, 8.2, 8.3, 8.4, 8.5_

- [x] 4.1 Write property test for 10-bit source produces 10-bit output
  - **Property 26: 10-bit source produces 10-bit output**
  - **Validates: Requirements 8.1**

- [x] 4.2 Write property test for 10-bit filter chain pixel format
  - **Property 28: 10-bit filter chain uses correct pixel format**
  - **Validates: Requirements 8.3**

- [x] 4.3 Write property test for 8-bit source produces 8-bit output
  - **Property 29: 8-bit source produces 8-bit output**
  - **Validates: Requirements 8.4**

- [x] 4.4 Write property test for unknown bit depth defaults to 10-bit
  - **Property 30: Unknown bit depth defaults to 10-bit**
  - **Validates: Requirements 8.5**

- [x] 5. Implement command builder for software AV1 encoding
  - Create CommandBuilder struct
  - Implement SVT-AV1 command generation with CRF mode
  - Implement libaom-av1 command generation with CRF mode
  - Implement librav1e command generation with CRF mode
  - Add filter chain construction (format conversion before encoder)
  - Add audio/subtitle stream copying (-c:a copy -c:s copy)
  - Add film-grain and tune parameters for SVT-AV1
  - _Requirements: 9.1, 9.2, 9.3, 9.4, 9.5_

- [x] 5.1 Write property test for audio and subtitle stream copying
  - **Property 31: Audio and subtitle stream copying**
  - **Validates: Requirements 9.4**

- [x] 5.2 Write property test for format filter before encoder
  - **Property 32: Format filter before encoder**
  - **Validates: Requirements 9.5**

- [x] 6. Implement test clip workflow for REMUX sources
  - Create TestClipWorkflow struct
  - Implement test clip extraction (30-60 seconds)
  - Implement scene selection heuristics (dark, grain, motion)
  - Implement test clip encoding with proposed parameters
  - Implement user approval loop (approve/adjust/reject)
  - Implement parameter adjustment (lower CRF by 2, slower preset by 1)
  - Add test clip skip logic for non-REMUX tiers
  - _Requirements: 4.1, 4.2, 4.3, 4.4, 4.5, 7.4_

- [x] 6.1 Write property test for REMUX sources trigger test clip extraction
  - **Property 16: REMUX sources trigger test clip extraction**
  - **Validates: Requirements 4.1**

- [x] 6.2 Write property test for test clip requires user approval
  - **Property 17: Test clip requires user approval**
  - **Validates: Requirements 4.3**

- [x] 6.3 Write property test for user feedback adjusts parameters
  - **Property 18: User feedback adjusts parameters**
  - **Validates: Requirements 4.4**

- [x] 6.4 Write property test for approved test clip parameters match full encode
  - **Property 19: Approved test clip parameters match full encode**
  - **Validates: Requirements 4.5**

- [x] 6.5 Write property test for LOW-QUALITY sources skip test clip
  - **Property 20: LOW-QUALITY sources skip test clip**
  - **Validates: Requirements 7.4**

- [x] 7. Implement skip re-encoding logic for clean WEB-DL sources
  - Add should_skip_encode method to SourceClassifier
  - Check for HEVC/AV1/VP9 codec in WEB-DL tier
  - Add user override flag to force re-encoding
  - Update job processing to skip when appropriate
  - _Requirements: 6.1_

- [x] 7.1 Write property test for skip re-encoding for clean modern codecs
  - **Property 15: Skip re-encoding for clean modern codecs**
  - **Validates: Requirements 6.1**

- [x] 8. Remove Docker dependencies from codebase
  - Remove Docker client library from Cargo.toml
  - Delete Docker-specific code from ffmpeg_docker.rs
  - Remove Docker initialization and health checks
  - Remove container lifecycle management code
  - Update error messages to remove Docker diagnostics
  - _Requirements: 10.1, 10.2, 10.3, 10.4, 10.5_

- [x] 8.1 Write property test for Docker-free execution
  - **Property 3: Docker-free execution**
  - **Validates: Requirements 10.2, 10.3, 10.4**

- [x] 8.2 Write property test for error messages exclude Docker diagnostics
  - **Property 4: Error messages exclude Docker diagnostics**
  - **Validates: Requirements 1.5**

- [x] 9. Implement direct FFmpeg/FFprobe execution
  - Replace Docker-based probe_file with direct FFprobe execution
  - Replace Docker-based encoding with direct FFmpeg execution
  - Implement subprocess spawning with proper error handling
  - Capture stdout/stderr from FFmpeg processes
  - Add timeout handling for stuck processes
  - _Requirements: 1.1, 1.5_

- [x] 9.1 Write property test for encoder selection logging
  - **Property 8: Encoder selection logging**
  - **Validates: Requirements 2.4**

- [x] 10. Implement quality-first decision logging
  - Add logging for CRF selection decisions
  - Add logging for preset selection decisions
  - Add logging for film-grain parameter decisions
  - Add logging for tier classification decisions
  - Include reasoning in log messages (why quality was prioritized)
  - _Requirements: 12.1, 12.2, 12.3, 12.4, 12.5_

- [x] 10.1 Write property test for quality prioritization in CRF selection
  - **Property 24: Quality prioritization in CRF selection**
  - **Validates: Requirements 12.2, 12.4**

- [x] 10.2 Write property test for quality prioritization in preset selection
  - **Property 25: Quality prioritization in preset selection**
  - **Validates: Requirements 12.3, 12.4**

- [x] 10.3 Write property test for quality decision logging
  - **Property 33: Quality decision logging**
  - **Validates: Requirements 12.5**

- [x] 11. Update configuration and job state models
  - Add new fields to TranscodeConfig (ffmpeg_bin, ffprobe_bin, etc.)
  - Add new fields to Job (quality_tier, crf_used, preset_used, encoder_used)
  - Remove Docker-related fields from TranscodeConfig
  - Update configuration loading and validation
  - Update job serialization/deserialization
  - _Requirements: 1.4, 11.1, 11.2, 11.3, 11.4, 11.5_

- [x] 12. Integrate all components into main daemon loop
  - Update job processing to use FFmpegManager
  - Update job processing to use enhanced SourceClassifier
  - Update job processing to use QualityCalculator
  - Update job processing to use TestClipWorkflow for REMUX
  - Update job processing to use CommandBuilder
  - Update job processing to skip clean WEB-DL sources
  - Wire up all error handling paths
  - _Requirements: All_

- [x] 13. Checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.

- [x] 14. Create installation documentation
  - Document FFmpeg 8.0+ requirement
  - Document required encoder libraries (libsvtav1, libaom, librav1e)
  - Provide FFmpeg build instructions with encoder flags
  - Document FFMPEG_BIN configuration option
  - Document bundled binary option (if applicable)
  - Provide system package manager installation commands
  - _Requirements: 11.1, 11.2, 11.3, 11.4, 11.5_

- [x] 15. Create migration guide for existing users
  - Document configuration changes (remove Docker settings)
  - Document new configuration options (ffmpeg_bin, etc.)
  - Provide step-by-step migration instructions
  - Document expected performance changes (slower CPU encoding)
  - Document quality improvements (quality-first approach)
  - Provide troubleshooting guide for common issues

- [x] 16. Final checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.
