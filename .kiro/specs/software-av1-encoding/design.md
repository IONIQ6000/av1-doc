# Design Document

## Overview

This design transforms the video encoding application from a Docker-based architecture to a native software-only AV1 encoding system using FFmpeg 8.0+. The system will execute FFmpeg directly as a subprocess, eliminating all Docker dependencies while maintaining quality-first encoding with CPU-based AV1 encoders (SVT-AV1, libaom-av1, librav1e).

The core principle is **maximum perceptual quality**: all encoding decisions prioritize visual fidelity over compression efficiency, file size, or encoding speed. The system classifies sources into quality tiers (REMUX, WEB-DL, LOW-QUALITY) and applies tier-specific CRF values and presets optimized for quality preservation.

## Architecture

### Current Architecture (Docker-based)

```
┌─────────────┐
│   Daemon    │
│  (Rust)     │
└──────┬──────┘
       │
       ├─ Spawns Docker containers
       │  └─ FFmpeg 8.0 (linuxserver image)
       │     └─ QSV hardware encoding (av1_qsv)
       │
       ├─ FFprobe via Docker
       └─ Job management
```

### New Architecture (Native FFmpeg)

```
┌─────────────────────────────────────┐
│           Daemon (Rust)             │
├─────────────────────────────────────┤
│  ┌──────────────────────────────┐   │
│  │  FFmpeg Manager              │   │
│  │  - Version validation (≥8.0) │   │
│  │  - Encoder detection         │   │
│  │  - Direct subprocess exec    │   │
│  └──────────────────────────────┘   │
│                                     │
│  ┌──────────────────────────────┐   │
│  │  Source Classifier           │   │
│  │  - REMUX tier detection      │   │
│  │  - WEB-DL tier detection     │   │
│  │  - LOW-QUALITY detection     │   │
│  └──────────────────────────────┘   │
│                                     │
│  ┌──────────────────────────────┐   │
│  │  Quality Calculator          │   │
│  │  - CRF selection by tier     │   │
│  │  - Preset selection          │   │
│  │  - Film grain parameters     │   │
│  └──────────────────────────────┘   │
│                                     │
│  ┌──────────────────────────────┐   │
│  │  Test Clip Workflow          │   │
│  │  - Scene extraction          │   │
│  │  - User approval loop        │   │
│  │  - Parameter adjustment      │   │
│  └──────────────────────────────┘   │
└─────────────────────────────────────┘
         │
         ├─ Direct FFmpeg execution
         │  └─ libsvtav1 / libaom-av1 / librav1e
         │
         ├─ Direct FFprobe execution
         └─ Job state persistence
```

## Components and Interfaces

### 1. FFmpeg Manager

**Responsibility**: Manage FFmpeg binary location, version validation, and encoder detection.

**Interface**:
```rust
pub struct FFmpegManager {
    ffmpeg_bin: PathBuf,
    ffprobe_bin: PathBuf,
    version: FFmpegVersion,
    available_encoders: Vec<AV1Encoder>,
}

pub enum AV1Encoder {
    SvtAv1Psy,
    SvtAv1,
    LibAom,
    LibRav1e,
}

impl FFmpegManager {
    /// Initialize manager, validate FFmpeg ≥ 8.0, detect encoders
    pub fn new(config: &TranscodeConfig) -> Result<Self>;
    
    /// Get the best available AV1 encoder
    pub fn best_encoder(&self) -> &AV1Encoder;
    
    /// Execute FFmpeg command directly
    pub async fn execute_ffmpeg(&self, args: Vec<String>) -> Result<FFmpegResult>;
    
    /// Execute FFprobe command directly
    pub async fn execute_ffprobe(&self, file_path: &Path) -> Result<FFProbeData>;
}
```

### 2. Source Classifier (Enhanced)

**Responsibility**: Classify video sources into quality tiers for appropriate encoding parameters.

**Interface**:
```rust
pub enum QualityTier {
    Remux,      // Blu-ray/UHD remux, high-bitrate masters
    WebDl,      // Streaming downloads, already encoded
    LowQuality, // Low-bitrate rips, already degraded
}

pub struct SourceClassification {
    pub tier: QualityTier,
    pub confidence: f64,
    pub reasons: Vec<String>,
    pub bitrate_per_pixel: Option<f64>,
}

impl SourceClassifier {
    /// Classify source into quality tier
    pub fn classify(&self, meta: &FFProbeData, file_path: &Path) -> SourceClassification;
    
    /// Check if source should skip re-encoding (clean HEVC/AV1/VP9)
    pub fn should_skip_encode(&self, classification: &SourceClassification, meta: &FFProbeData) -> bool;
}
```

**Classification Logic**:
- **REMUX tier**: bitrate > 15 Mbps (1080p) or > 40 Mbps (2160p), OR lossless audio codecs (TrueHD, DTS, FLAC)
- **WEB-DL tier**: HEVC/AV1/VP9 codec with clean quality, OR filename contains WEB-DL/WEBRIP markers
- **LOW-QUALITY tier**: bitrate < 5 Mbps (1080p), OR visible artifacts, OR low bits-per-pixel

### 3. Quality Calculator

**Responsibility**: Determine CRF, preset, and encoding parameters based on source tier and properties.

**Interface**:
```rust
pub struct EncodingParams {
    pub crf: u8,
    pub preset: u8,
    pub tune: Option<u8>,
    pub film_grain: Option<u8>,
    pub bit_depth: BitDepth,
    pub pixel_format: String,
}

impl QualityCalculator {
    /// Calculate encoding parameters for a source
    pub fn calculate_params(
        &self,
        classification: &SourceClassification,
        meta: &FFProbeData,
        encoder: &AV1Encoder,
    ) -> EncodingParams;
}
```

**CRF Selection Logic** (SVT-AV1):
- **REMUX 1080p**: CRF 18 (range 16-21)
- **REMUX 2160p**: CRF 20 (range 18-22)
- **WEB-DL 1080p**: CRF 26 (range 24-29)
- **WEB-DL 2160p**: CRF 28 (range 26-32)
- **LOW-QUALITY**: CRF 30 (range 30-35)

**Preset Selection Logic** (SVT-AV1):
- **REMUX**: preset 2-4 (default 3, slower for grainy content)
- **WEB-DL**: preset 4-6 (default 5)
- **LOW-QUALITY**: preset 6-8

**Film Grain Logic**:
- Enable for REMUX tier with visible grain: `-svtav1-params film-grain=8`
- Enable tune=3 for grain-optimized encoding (SVT-AV1-PSY)
- Disable for WEB-DL and LOW-QUALITY tiers

### 4. Test Clip Workflow

**Responsibility**: Extract and encode test clips for REMUX sources, await user approval.

**Interface**:
```rust
pub struct TestClipWorkflow {
    temp_dir: PathBuf,
}

impl TestClipWorkflow {
    /// Extract test clip from source
    pub async fn extract_test_clip(
        &self,
        source: &Path,
        meta: &FFProbeData,
    ) -> Result<TestClipInfo>;
    
    /// Encode test clip with proposed parameters
    pub async fn encode_test_clip(
        &self,
        clip_info: &TestClipInfo,
        params: &EncodingParams,
        ffmpeg_mgr: &FFmpegManager,
    ) -> Result<PathBuf>;
    
    /// Wait for user approval or adjustment request
    pub async fn await_user_approval(&self) -> Result<ApprovalDecision>;
}

pub enum ApprovalDecision {
    Approved,
    LowerCrf(u8),      // User requests lower CRF (higher quality)
    SlowerPreset(u8),  // User requests slower preset
    Rejected,
}
```

**Test Clip Selection**:
- Duration: 30-60 seconds
- Scene selection criteria:
  - Darkest scene (shadows reveal banding)
  - Most grain/texture (tests grain preservation)
  - Highest motion (tests temporal compression)
- Extract using: `ffmpeg -ss {START} -t {DURATION} -i "{INPUT}" -c copy "{TEST_CLIP}"`

### 5. Command Builder

**Responsibility**: Generate FFmpeg command lines for software AV1 encoding.

**Interface**:
```rust
impl CommandBuilder {
    /// Build full encode command
    pub fn build_encode_command(
        &self,
        input: &Path,
        output: &Path,
        params: &EncodingParams,
        encoder: &AV1Encoder,
        meta: &FFProbeData,
    ) -> Vec<String>;
    
    /// Build test clip extraction command
    pub fn build_test_clip_command(
        &self,
        input: &Path,
        output: &Path,
        start_time: f64,
        duration: f64,
    ) -> Vec<String>;
}
```

**Command Template** (SVT-AV1):
```bash
ffmpeg -i "{INPUT}" \
  -map 0:v:0 -map 0:a? -map 0:s? \
  -vf "format=yuv420p10le" \
  -c:v libsvtav1 -crf {CRF} -preset {PRESET} \
  -pix_fmt yuv420p10le \
  -svtav1-params tune={TUNE}:film-grain={GRAIN} \
  -c:a copy -c:s copy \
  "{OUTPUT}"
```

## Data Models

### Configuration

```rust
pub struct TranscodeConfig {
    // Existing fields...
    
    // New fields for software encoding
    pub ffmpeg_bin: PathBuf,           // Path to FFmpeg binary (default: "ffmpeg")
    pub ffprobe_bin: PathBuf,          // Path to FFprobe binary (default: "ffprobe")
    pub require_ffmpeg_version: String, // Minimum version (default: "8.0")
    pub preferred_encoder: Option<AV1Encoder>, // User override for encoder selection
    pub enable_test_clip_workflow: bool, // Enable test clips for REMUX (default: true)
    pub test_clip_duration: u64,       // Test clip duration in seconds (default: 45)
}
```

### Job State (Enhanced)

```rust
pub struct Job {
    // Existing fields...
    
    // New fields for software encoding
    pub quality_tier: Option<QualityTier>,
    pub crf_used: Option<u8>,
    pub preset_used: Option<u8>,
    pub encoder_used: Option<String>,
    pub test_clip_path: Option<PathBuf>,
    pub test_clip_approved: Option<bool>,
}
```

## Correctness Properties

*A property is a characteristic or behavior that should hold true across all valid executions of a system-essentially, a formal statement about what the system should do. Properties serve as the bridge between human-readable specifications and machine-verifiable correctness guarantees.*

### Property Reflection

After analyzing all acceptance criteria, I identified several areas of redundancy:

1. **CRF selection properties** (5.1, 5.2, 6.2, 6.3, 7.1) can be consolidated into a single property that tests CRF selection across all tiers and resolutions
2. **Preset selection properties** (5.3, 6.4, 7.2) can be consolidated into a single property that tests preset selection across all tiers
3. **Film-grain properties** (6.5, 7.3) are redundant - both test that non-REMUX tiers disable film-grain
4. **Command generation properties** (9.1, 9.2) are subsumed by the consolidated CRF/preset properties
5. **Docker removal properties** (10.2, 10.3, 10.4) all test the same thing - that Docker is not used
6. **Quality priority properties** (12.2, 12.3) are redundant with the consolidated CRF/preset properties

After consolidation, we have 30 unique properties that provide comprehensive validation coverage.

### FFmpeg Version and Initialization

**Property 1: FFmpeg version validation**
*For any* FFmpeg binary, if its reported version is less than 8.0, initialization SHALL fail with a version error, and if version is 8.0 or greater, initialization SHALL succeed
**Validates: Requirements 1.2, 1.3**

**Property 2: FFmpeg binary path configuration**
*For any* valid file path specified in FFMPEG_BIN configuration, the system SHALL use that path for FFmpeg execution instead of PATH lookup
**Validates: Requirements 1.4**

**Property 3: Docker-free execution**
*For any* encoding job, the system SHALL spawn FFmpeg processes directly without invoking docker commands (run, pull, build) or checking Docker daemon availability
**Validates: Requirements 10.2, 10.3, 10.4**

**Property 4: Error messages exclude Docker diagnostics**
*For any* FFmpeg execution error, the error message SHALL not contain Docker-specific strings (container, image, daemon, docker)
**Validates: Requirements 1.5**

### Encoder Detection and Selection

**Property 5: Encoder detection at startup**
*For any* system initialization, the encoder detection SHALL query FFmpeg for available encoders and parse the output to identify AV1 software encoders
**Validates: Requirements 2.1**

**Property 6: Encoder priority selection**
*For any* set of available AV1 encoders, the system SHALL select the highest priority encoder: SVT-AV1-PSY > libsvtav1 > libaom-av1 > librav1e
**Validates: Requirements 2.2**

**Property 7: Missing encoder error**
*For any* system initialization where no AV1 software encoders are detected, initialization SHALL fail with an error listing required encoder libraries
**Validates: Requirements 2.3**

**Property 8: Encoder selection logging**
*For any* successful encoder selection, the system SHALL log the selected encoder name
**Validates: Requirements 2.4**

**Property 9: SVT-AV1-PSY perceptual tuning**
*For any* encoding job where SVT-AV1-PSY is the selected encoder, the command SHALL include perceptual tuning parameters (tune=3)
**Validates: Requirements 2.5, 5.5**

### Source Classification

**Property 10: Classification produces valid tier**
*For any* video file metadata, source classification SHALL produce exactly one of: REMUX, WEB-DL, or LOW-QUALITY tier
**Validates: Requirements 3.1**

**Property 11: High bitrate REMUX classification**
*For any* 1080p source with bitrate > 15 Mbps OR 2160p source with bitrate > 40 Mbps, classification SHALL produce REMUX tier
**Validates: Requirements 3.2**

**Property 12: Modern codec WEB-DL classification**
*For any* source with codec HEVC, AV1, or VP9, classification SHALL produce WEB-DL tier (unless bitrate indicates REMUX)
**Validates: Requirements 3.3**

**Property 13: Low bitrate LOW-QUALITY classification**
*For any* 1080p source with bitrate < 5 Mbps, classification SHALL produce LOW-QUALITY tier
**Validates: Requirements 3.4**

**Property 14: Uncertain classification defaults to higher tier**
*For any* source where classification confidence is below threshold, the system SHALL select the higher quality tier between ambiguous options
**Validates: Requirements 3.5**

**Property 15: Skip re-encoding for clean modern codecs**
*For any* WEB-DL source already encoded with HEVC, AV1, or VP9, the system SHALL skip re-encoding unless user explicitly requests it
**Validates: Requirements 6.1**

### Test Clip Workflow

**Property 16: REMUX sources trigger test clip extraction**
*For any* source classified as REMUX tier, the system SHALL extract a test clip of 30-60 seconds before starting full encode
**Validates: Requirements 4.1**

**Property 17: Test clip requires user approval**
*For any* test clip encoding completion, the system SHALL pause and wait for user input before proceeding to full encode
**Validates: Requirements 4.3**

**Property 18: User feedback adjusts parameters**
*For any* user rejection of test clip quality, the system SHALL lower CRF by 2 points OR reduce preset speed by 1 step
**Validates: Requirements 4.4**

**Property 19: Approved test clip parameters match full encode**
*For any* user-approved test clip, the full encode SHALL use identical CRF, preset, and tuning parameters
**Validates: Requirements 4.5**

**Property 20: LOW-QUALITY sources skip test clip**
*For any* source classified as LOW-QUALITY tier, the system SHALL not extract or encode a test clip
**Validates: Requirements 7.4**

### Quality Parameter Selection

**Property 21: CRF selection by tier and resolution**
*For any* source, the CRF value SHALL be: (REMUX 1080p: 18), (REMUX 2160p: 20), (WEB-DL 1080p: 26), (WEB-DL 2160p: 28), (LOW-QUALITY: 30)
**Validates: Requirements 5.1, 5.2, 6.2, 6.3, 7.1**

**Property 22: Preset selection by tier**
*For any* source, the preset value SHALL be: (REMUX: ≤3), (WEB-DL: 5), (LOW-QUALITY: ≥6)
**Validates: Requirements 5.3, 6.4, 7.2**

**Property 23: Film-grain enabled for REMUX only**
*For any* REMUX source, film-grain synthesis SHALL be enabled (value 8), and for any WEB-DL or LOW-QUALITY source, film-grain SHALL be disabled
**Validates: Requirements 6.5, 7.3**

**Property 24: Quality prioritization in CRF selection**
*For any* encoding decision, when choosing between CRF values, the system SHALL select the lower CRF (higher quality) unless user explicitly requests size optimization
**Validates: Requirements 12.2, 12.4**

**Property 25: Quality prioritization in preset selection**
*For any* encoding decision, when choosing between presets, the system SHALL select the slower preset (higher quality) unless user explicitly requests speed optimization
**Validates: Requirements 12.3, 12.4**

### Bit Depth and Color Handling

**Property 26: 10-bit source produces 10-bit output**
*For any* source with 10-bit color depth, the output SHALL be 10-bit AV1 with yuv420p10le pixel format
**Validates: Requirements 8.1**

**Property 27: HDR metadata preservation**
*For any* source containing HDR metadata (PQ, HLG, bt2020), the output container SHALL preserve the HDR metadata
**Validates: Requirements 8.2**

**Property 28: 10-bit filter chain uses correct pixel format**
*For any* 10-bit source, the filter chain SHALL include format=yuv420p10le or format=p010le before the encoder
**Validates: Requirements 8.3**

**Property 29: 8-bit source produces 8-bit output**
*For any* source with 8-bit color depth, the output SHALL be 8-bit AV1 without upconverting to 10-bit
**Validates: Requirements 8.4**

**Property 30: Unknown bit depth defaults to 10-bit**
*For any* source where bit depth cannot be determined, the system SHALL default to 10-bit output to avoid quality loss
**Validates: Requirements 8.5**

### Command Generation

**Property 31: Audio and subtitle stream copying**
*For any* encode command, the command SHALL include "-c:a copy" and "-c:s copy" to preserve audio and subtitle streams without re-encoding
**Validates: Requirements 9.4**

**Property 32: Format filter before encoder**
*For any* encode command, the filter chain SHALL include format conversion (format=yuv420p10le or format=yuv420p) before the encoder input
**Validates: Requirements 9.5**

**Property 33: Quality decision logging**
*For any* encoding decision that prioritizes quality over efficiency, the system SHALL log the decision with explanation
**Validates: Requirements 12.5**



## Error Handling

### FFmpeg Version Errors

**Scenario**: FFmpeg not found or version < 8.0
**Handling**:
- Detect during initialization via `ffmpeg -version` command
- Parse version string to extract major.minor version
- If version < 8.0 or command fails:
  - Log clear error: "FFmpeg 8.0 or later required, found: {version}"
  - Terminate application with exit code 1
  - Do not attempt to start daemon or process jobs

### Encoder Detection Errors

**Scenario**: No AV1 software encoders available
**Handling**:
- Query encoders via `ffmpeg -hide_banner -encoders | grep -E "libsvtav1|libaom-av1|librav1e"`
- If no matches found:
  - Log error: "No AV1 software encoders detected. Required: libsvtav1, libaom-av1, or librav1e"
  - Provide installation guidance in error message
  - Terminate application with exit code 1

### Encoding Errors

**Scenario**: FFmpeg encoding process fails
**Handling**:
- Capture stderr from FFmpeg process
- Parse stderr for common error patterns:
  - Codec not found → suggest encoder installation
  - Invalid pixel format → check bit depth detection
  - Filter chain errors → log filter string for debugging
- Mark job as Failed with reason from stderr
- Continue processing other jobs (don't crash daemon)

### Test Clip Errors

**Scenario**: Test clip extraction or encoding fails
**Handling**:
- If extraction fails: log error, skip test clip workflow, proceed to full encode
- If test clip encoding fails: log error, ask user whether to proceed with full encode
- Never block full encode indefinitely on test clip failure

### File System Errors

**Scenario**: Cannot write output file, temp directory full
**Handling**:
- Check disk space before starting encode
- If insufficient space: mark job as Failed with "Insufficient disk space" reason
- If write fails during encode: clean up partial output, mark job as Failed
- Retry logic: do not retry file system errors (user must fix)

## Testing Strategy

### Unit Testing

Unit tests will verify specific examples and edge cases:

1. **FFmpeg version parsing**: Test parsing of various version strings (8.0, 8.1.2, 7.0, invalid)
2. **Encoder detection parsing**: Test parsing of ffmpeg -encoders output with different encoder combinations
3. **Source classification**: Test classification with known metadata examples (high bitrate, low bitrate, various codecs)
4. **CRF calculation**: Test CRF selection for specific tier/resolution combinations
5. **Command building**: Test that generated commands contain expected arguments
6. **Error message validation**: Test that error messages don't contain Docker strings

### Property-Based Testing

Property-based tests will verify universal properties across all inputs using the **proptest** crate (Rust's property testing library). Each property test will run a minimum of 100 iterations with randomly generated inputs.

**Configuration**:
```rust
#![proptest_config(ProptestConfig::with_cases(100))]
```

**Property Test Structure**:
Each property-based test MUST:
1. Be tagged with a comment referencing the design document property
2. Use format: `/// **Feature: software-av1-encoding, Property {N}: {property_text}**`
3. Generate random inputs using proptest strategies
4. Assert the property holds for all generated inputs

**Example Property Test**:
```rust
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
        resolution in prop_oneof![
            Just((1920, 1080)),
            Just((3840, 2160)),
        ],
    ) {
        let expected_crf = match (tier, resolution) {
            (QualityTier::Remux, (1920, 1080)) => 18,
            (QualityTier::Remux, (3840, 2160)) => 20,
            (QualityTier::WebDl, (1920, 1080)) => 26,
            (QualityTier::WebDl, (3840, 2160)) => 28,
            (QualityTier::LowQuality, _) => 30,
            _ => panic!("Unexpected combination"),
        };
        
        let params = calculate_encoding_params(tier, resolution.0, resolution.1);
        
        prop_assert_eq!(
            params.crf,
            expected_crf,
            "CRF for {:?} at {}x{} should be {}, got {}",
            tier, resolution.0, resolution.1, expected_crf, params.crf
        );
    }
}
```

**Property Test Coverage**:
- Properties 1-33 from the Correctness Properties section
- Each property maps to one or more property-based tests
- Tests generate random inputs (versions, metadata, configurations)
- Tests verify properties hold across all generated inputs

### Integration Testing

Integration tests will verify end-to-end workflows:

1. **Full encode workflow**: Test complete encode from source file to output file
2. **Test clip workflow**: Test extraction, encoding, approval loop
3. **Multi-tier encoding**: Test encoding sources from all three quality tiers
4. **Bit depth preservation**: Test 8-bit and 10-bit sources produce correct outputs
5. **Encoder fallback**: Test that system works with different available encoders

Integration tests will use real FFmpeg binaries and test video files.

## Implementation Notes

### Migration Strategy

1. **Phase 1: Add native FFmpeg support alongside Docker**
   - Add FFmpegManager component
   - Add configuration flag: `use_docker: bool`
   - Keep existing Docker code functional
   - Test native path thoroughly

2. **Phase 2: Remove Docker dependencies**
   - Remove Docker client library from Cargo.toml
   - Delete Docker-related code from ffmpeg_docker.rs
   - Rename ffmpeg_docker.rs to ffmpeg_native.rs
   - Update all imports and references

3. **Phase 3: Add quality-first features**
   - Implement enhanced source classification
   - Implement test clip workflow
   - Implement tier-specific CRF/preset selection
   - Add film-grain synthesis support

### Performance Considerations

**CPU Encoding Speed**:
- SVT-AV1 preset 3 (REMUX): ~0.5-1 fps on modern CPU (1080p)
- SVT-AV1 preset 5 (WEB-DL): ~2-4 fps on modern CPU (1080p)
- Encoding times will be significantly longer than QSV hardware encoding
- Users should expect 10-20x longer encoding times vs hardware

**Parallelization**:
- SVT-AV1 automatically uses multiple CPU cores
- No need for manual thread configuration
- System should limit concurrent encodes to avoid CPU saturation
- Recommendation: 1 encode at a time for quality-first approach

### Compatibility

**FFmpeg 8.0 Features**:
- Improved SVT-AV1 integration
- Better 10-bit handling
- Enhanced HDR metadata support
- Dolby Vision stripping improvements

**Encoder Availability**:
- SVT-AV1: Most widely available, best performance
- libaom-av1: Reference encoder, slower but highest quality
- librav1e: Rust-based, good quality, moderate speed
- SVT-AV1-PSY: Fork with perceptual tuning, requires separate build

### Configuration Migration

**Old Config** (Docker-based):
```toml
docker_image = "lscr.io/linuxserver/ffmpeg:version-8.0-cli"
docker_bin = "docker"
gpu_device = "/dev/dri"
```

**New Config** (Native):
```toml
ffmpeg_bin = "ffmpeg"  # or "/usr/local/bin/ffmpeg"
ffprobe_bin = "ffprobe"
require_ffmpeg_version = "8.0"
enable_test_clip_workflow = true
test_clip_duration = 45
```

Users will need to:
1. Install FFmpeg 8.0+ with AV1 encoder support
2. Update configuration file to remove Docker settings
3. Add FFmpeg binary paths if not in PATH
4. Restart daemon

## Deployment Considerations

### System Requirements

**Minimum**:
- FFmpeg 8.0 or later
- At least one AV1 software encoder (libsvtav1, libaom-av1, or librav1e)
- 4+ CPU cores (8+ recommended for reasonable speed)
- 8GB+ RAM (16GB+ recommended for 4K content)

**Recommended**:
- FFmpeg 8.0+ with SVT-AV1-PSY
- 16+ CPU cores for faster encoding
- 32GB+ RAM for 4K content
- Fast NVMe storage for temp files

### Installation Options

**Option 1: System Package Manager**
```bash
# Ubuntu/Debian (if FFmpeg 8.0+ available)
apt install ffmpeg libsvtav1enc-dev

# Arch Linux
pacman -S ffmpeg svt-av1

# macOS
brew install ffmpeg
```

**Option 2: Build from Source**
```bash
# Build FFmpeg with SVT-AV1 support
./configure --enable-libsvtav1 --enable-libaom --enable-librav1e
make -j$(nproc)
make install
```

**Option 3: Static Binary**
- Download pre-built FFmpeg 8.0+ static binary
- Place in application directory
- Configure `ffmpeg_bin` to point to binary

### Monitoring and Logging

**Key Metrics to Log**:
- FFmpeg version and available encoders (at startup)
- Source classification decisions (per job)
- CRF and preset selections (per job)
- Encoding speed (fps) and duration (per job)
- File size reduction percentage (per job)
- Test clip approval/rejection (per REMUX job)

**Log Levels**:
- ERROR: FFmpeg failures, missing encoders, version errors
- WARN: Classification uncertainty, test clip failures
- INFO: Job start/complete, encoder selection, quality decisions
- DEBUG: FFmpeg command lines, detailed parameters

## Future Enhancements

### Potential Improvements

1. **Automatic scene detection for test clips**: Use FFmpeg scene detection to find optimal test clip segments
2. **Visual quality metrics**: Integrate VMAF or SSIM to automatically validate test clip quality
3. **Adaptive CRF**: Adjust CRF based on source complexity (grain, motion, detail)
4. **Multi-pass encoding**: Add optional two-pass mode for maximum quality
5. **Encoder-specific tuning**: Add libaom-av1 and librav1e specific quality parameters
6. **Parallel test clips**: Extract and encode multiple test clips simultaneously
7. **Quality presets**: Add user-selectable quality profiles (archival, balanced, efficient)

### Non-Goals

This design explicitly does NOT include:
- Hardware encoding (QSV, NVENC, AMF) - removed by design
- Docker support - removed by design
- Automatic quality assessment - requires user review
- Real-time encoding - quality-first approach is slow
- Streaming output - file-based workflow only
