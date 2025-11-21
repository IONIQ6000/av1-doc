# Design Document

## Overview

This design migrates the AV1 hardware encoding implementation from VAAPI to Intel QSV (Quick Sync Video) to enable 10-bit AV1 encoding support on Intel Arc GPUs. Testing has revealed that while VAAPI successfully encodes 8-bit AV1 content, it does not expose 10-bit AV1 encoding capabilities on Intel Arc A310/A380 GPUs. Intel QSV provides full support for both 8-bit and 10-bit AV1 hardware encoding through the `av1_qsv` codec.

The migration involves:
- Changing hardware device initialization from VAAPI to QSV
- Updating codec from `av1_vaapi` to `av1_qsv`
- Replacing quality parameter from `-qp` to `-global_quality`
- Updating Docker image to `lscr.io/linuxserver/ffmpeg:version-8.0-cli`
- Adding `LIBVA_DRIVER_NAME=iHD` environment variable
- Adjusting filter chain for QSV compatibility
- Updating AV1 profile handling (QSV uses profile 0 for both 8-bit and 10-bit)

## Architecture

### Current Architecture (VAAPI)

```
┌─────────────────┐
│  Rust Daemon    │
│                 │
│  ┌───────────┐  │
│  │ FFmpeg    │  │
│  │ Docker    │  │
│  │ Command   │  │
│  │ Builder   │  │
│  └─────┬─────┘  │
└────────┼────────┘
         │
         ▼
┌─────────────────────────────────┐
│  Docker Container               │
│  ghcr.io/linuxserver/ffmpeg     │
│                                 │
│  ffmpeg -init_hw_device         │
│    vaapi=va:/dev/dri/renderD128 │
│  -hwaccel vaapi                 │
│  -vf format=nv12,hwupload       │
│  -c:v av1_vaapi                 │
│  -qp 30 -profile:v 0            │
└─────────────────────────────────┘
         │
         ▼
┌─────────────────┐
│  /dev/dri       │
│  renderD128     │
│  (Intel Arc GPU)│
│  VAAPI Driver   │
│  ✅ 8-bit AV1   │
│  ❌ 10-bit AV1  │
└─────────────────┘
```

### New Architecture (QSV)

```
┌─────────────────┐
│  Rust Daemon    │
│                 │
│  ┌───────────┐  │
│  │ FFmpeg    │  │
│  │ Docker    │  │
│  │ Command   │  │
│  │ Builder   │  │
│  └─────┬─────┘  │
└────────┼────────┘
         │
         ▼
┌──────────────────────────────────────┐
│  Docker Container                    │
│  lscr.io/linuxserver/ffmpeg:v8.0-cli │
│  ENV: LIBVA_DRIVER_NAME=iHD          │
│                                      │
│  ffmpeg -init_hw_device              │
│    qsv=hw:/dev/dri/renderD128        │
│  -filter_hw_device hw                │
│  -vf format=p010le,hwupload          │
│  -c:v av1_qsv                        │
│  -global_quality 30 -profile:v main  │
└──────────────────────────────────────┘
         │
         ▼
┌─────────────────┐
│  /dev/dri       │
│  renderD128     │
│  (Intel Arc GPU)│
│  QSV/VPL Driver │
│  ✅ 8-bit AV1   │
│  ✅ 10-bit AV1  │
└─────────────────┘
```

## Components and Interfaces

### 1. FFmpeg Command Builder (`run_av1_qsv_job`)

**Purpose**: Construct and execute ffmpeg commands using Intel QSV for AV1 encoding

**Key Changes**:
- Rename function from `run_av1_vaapi_job` to `run_av1_qsv_job`
- Change hardware device initialization
- Update codec selection
- Modify quality parameter
- Add environment variable
- Adjust filter chain

**Interface**:
```rust
pub async fn run_av1_qsv_job(
    cfg: &TranscodeConfig,
    input: &Path,
    temp_output: &Path,
    meta: &FFProbeData,
    decision: &WebSourceDecision,
    encoding_params: &EncodingParams,
) -> Result<FFmpegResult>
```

### 2. Encoding Parameters (`determine_encoding_params`)

**Purpose**: Determine optimal encoding parameters based on source analysis

**Key Changes**:
- Update AV1 profile selection: QSV uses profile 0 (main) for both 8-bit and 10-bit
- Keep pixel format selection unchanged (nv12 for 8-bit, p010le for 10-bit)
- Keep QP calculation unchanged

**Interface**:
```rust
pub fn determine_encoding_params(
    meta: &FFProbeData,
    input_file: &Path,
) -> EncodingParams
```

**Profile Mapping**:
- VAAPI: Profile 0 = 8-bit, Profile 1 = 10-bit
- QSV: Profile 0 (main) = both 8-bit and 10-bit

### 3. Configuration (`TranscodeConfig`)

**Purpose**: Store system configuration including Docker image

**Key Changes**:
- Update default `docker_image` to `"lscr.io/linuxserver/ffmpeg:version-8.0-cli"`
- Keep `gpu_device` unchanged (`/dev/dri`)
- Maintain backward compatibility with existing configs

**Interface**: No changes to struct definition, only default value

### 4. Quality Calculation (`calculate_optimal_qp`)

**Purpose**: Calculate optimal quality parameter for encoding

**Key Changes**: None - function remains unchanged, QP value is used directly as `global_quality`

## Data Models

### EncodingParams

```rust
#[derive(Debug, Clone)]
pub struct EncodingParams {
    pub bit_depth: BitDepth,
    pub pixel_format: String,  // "nv12" or "p010le"
    pub av1_profile: u8,        // 0 for QSV (both 8-bit and 10-bit)
    pub qp: i32,                // Quality value (20-40)
    pub is_hdr: bool,
}
```

**Changes**:
- `av1_profile`: Always 0 for QSV (was 0 for 8-bit, 1 for 10-bit in VAAPI)
- Other fields remain unchanged

### FFmpegResult

```rust
#[derive(Debug)]
pub struct FFmpegResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub quality_used: i32,  // QP/global_quality value used
}
```

**Changes**: None

## Correctness Properties

*A property is a characteristic or behavior that should hold true across all valid executions of a system-essentially, a formal statement about what the system should do. Properties serve as the bridge between human-readable specifications and machine-verifiable correctness guarantees.*

### Property 1: QSV Hardware Initialization

*For any* encoding job, the hardware device initialization string SHALL contain "qsv=hw:/dev/dri/renderD128" instead of "vaapi=va:/dev/dri/renderD128"

**Validates: Requirements 1.1, 3.1**

### Property 2: AV1 QSV Codec Selection

*For any* encoding job, the video codec argument SHALL be "av1_qsv" instead of "av1_vaapi"

**Validates: Requirements 1.2**

### Property 3: Global Quality Parameter

*For any* encoding job with a calculated quality value, the quality parameter SHALL be "-global_quality <value>" instead of "-qp <value>"

**Validates: Requirements 1.3, 4.2**

### Property 4: LIBVA Driver Environment Variable

*For any* Docker command, the environment variables SHALL include "LIBVA_DRIVER_NAME=iHD"

**Validates: Requirements 1.4**

### Property 5: 10-bit Pixel Format Selection

*For any* source with 10-bit color depth, the encoding parameters SHALL specify "p010le" as the pixel format

**Validates: Requirements 2.1**

### Property 6: QSV Profile for 10-bit

*For any* 10-bit source encoded with QSV, the AV1 profile SHALL be 0 (main) instead of 1

**Validates: Requirements 2.2**

### Property 7: 8-bit Pixel Format Selection

*For any* source with 8-bit color depth, the encoding parameters SHALL specify "nv12" as the pixel format

**Validates: Requirements 2.4**

### Property 8: Device Path in Initialization

*For any* QSV hardware initialization, the device path SHALL be "/dev/dri/renderD128"

**Validates: Requirements 3.1**

### Property 9: Docker Device Mounting

*For any* Docker command, the device mounting SHALL include "/dev/dri:/dev/dri"

**Validates: Requirements 3.2**

### Property 10: Quality Calculation Preservation

*For any* source file with given metadata, the calculated quality value SHALL be identical before and after migration

**Validates: Requirements 4.1**

### Property 11: Quality Value in Result

*For any* completed encoding job, the FFmpegResult SHALL contain the quality value that was used

**Validates: Requirements 4.4**

### Property 12: Filter Chain Ordering

*For any* filter chain, the format conversion filter SHALL appear before the hwupload filter

**Validates: Requirements 5.1**

### Property 13: QSV HWUpload Filter

*For any* QSV encoding job, the hwupload filter SHALL not include the "extra_hw_frames" parameter

**Validates: Requirements 5.2**

### Property 14: 10-bit Filter Chain

*For any* 10-bit source, the filter chain SHALL include "format=p010le" before "hwupload"

**Validates: Requirements 5.3**

### Property 15: 8-bit Filter Chain

*For any* 8-bit source, the filter chain SHALL include "format=nv12" before "hwupload"

**Validates: Requirements 5.4**

## Error Handling

### Hardware Initialization Errors

**Scenario**: QSV device initialization fails

**Handling**:
- Log error with device path: `/dev/dri/renderD128`
- Include QSV-specific error message from ffmpeg stderr
- Return error with context about QSV initialization failure

**Example Error Message**:
```
Failed to initialize QSV hardware device at /dev/dri/renderD128
FFmpeg error: Cannot load libmfx...
```

### Encoding Errors

**Scenario**: AV1 QSV encoding fails during execution

**Handling**:
- Capture ffmpeg stderr output
- Log encoding parameters used (bit depth, profile, quality)
- Return FFmpegResult with non-zero exit code
- Include full stderr in result for debugging

### Configuration Errors

**Scenario**: Docker image not available or incompatible

**Handling**:
- Log warning if docker_image doesn't match recommended version
- Attempt to use configured image anyway (backward compatibility)
- If Docker pull fails, return error with image name

### Filter Chain Errors

**Scenario**: Hardware upload filter fails

**Handling**:
- Log filter chain that was attempted
- Include ffmpeg error about filter initialization
- Suggest checking GPU device permissions

## Testing Strategy

### Unit Testing

**Test Coverage**:
1. **Command Construction Tests**
   - Verify QSV initialization string format
   - Verify av1_qsv codec selection
   - Verify global_quality parameter format
   - Verify LIBVA_DRIVER_NAME environment variable
   - Verify filter chain construction for 8-bit and 10-bit

2. **Parameter Determination Tests**
   - Test profile selection for 8-bit sources (should be 0)
   - Test profile selection for 10-bit sources (should be 0)
   - Test pixel format selection for 8-bit (should be nv12)
   - Test pixel format selection for 10-bit (should be p010le)

3. **Quality Calculation Tests**
   - Verify QP calculation remains unchanged
   - Test QP value range (20-40)
   - Test QP adjustments for different codecs and resolutions

4. **Configuration Tests**
   - Test default docker_image value
   - Test backward compatibility with old configs
   - Test gpu_device path handling

### Property-Based Testing

**Framework**: Use `proptest` crate for Rust property-based testing

**Configuration**: Each property test should run a minimum of 100 iterations

**Test Properties**:

1. **Property Test: QSV Initialization Format**
   - **Feature: av1-qsv-migration, Property 1: QSV Hardware Initialization**
   - Generate: Random encoding parameters
   - Verify: Command contains "qsv=hw:/dev/dri/renderD128"
   - Verify: Command does not contain "vaapi=va"

2. **Property Test: Codec Selection**
   - **Feature: av1-qsv-migration, Property 2: AV1 QSV Codec Selection**
   - Generate: Random encoding parameters
   - Verify: Command contains "-c:v av1_qsv"
   - Verify: Command does not contain "av1_vaapi"

3. **Property Test: Quality Parameter Format**
   - **Feature: av1-qsv-migration, Property 3: Global Quality Parameter**
   - Generate: Random quality values (20-40)
   - Verify: Command contains "-global_quality <value>"
   - Verify: Command does not contain "-qp"

4. **Property Test: Environment Variable**
   - **Feature: av1-qsv-migration, Property 4: LIBVA Driver Environment Variable**
   - Generate: Random encoding parameters
   - Verify: Docker command includes "-e LIBVA_DRIVER_NAME=iHD"

5. **Property Test: Pixel Format Selection**
   - **Feature: av1-qsv-migration, Property 5 & 7: Pixel Format Selection**
   - Generate: Random bit depths (8-bit, 10-bit)
   - Verify: 8-bit → nv12, 10-bit → p010le

6. **Property Test: Profile Selection**
   - **Feature: av1-qsv-migration, Property 6: QSV Profile for 10-bit**
   - Generate: Random bit depths
   - Verify: All sources use profile 0 for QSV

7. **Property Test: Filter Chain Ordering**
   - **Feature: av1-qsv-migration, Property 12: Filter Chain Ordering**
   - Generate: Random encoding parameters
   - Verify: "format=" appears before "hwupload" in filter string

8. **Property Test: HWUpload Parameters**
   - **Feature: av1-qsv-migration, Property 13: QSV HWUpload Filter**
   - Generate: Random encoding parameters
   - Verify: "hwupload" does not contain "extra_hw_frames"

9. **Property Test: Quality Calculation Consistency**
   - **Feature: av1-qsv-migration, Property 10: Quality Calculation Preservation**
   - Generate: Random source metadata
   - Verify: calculate_optimal_qp returns same value regardless of encoding method

10. **Property Test: Result Quality Value**
    - **Feature: av1-qsv-migration, Property 11: Quality Value in Result**
    - Generate: Random encoding parameters
    - Verify: FFmpegResult.quality_used matches input quality parameter

### Integration Testing

**Manual Testing Required**:
1. Test actual 8-bit AV1 encoding with QSV
2. Test actual 10-bit AV1 encoding with QSV
3. Verify output file pixel format (yuv420p for 8-bit, yuv420p10le for 10-bit)
4. Compare encoding speed between VAAPI (8-bit only) and QSV
5. Verify GPU utilization during encoding
6. Test with various source codecs (H.264, HEVC, VP9, AV1)
7. Test with various resolutions (720p, 1080p, 4K)

**Integration Test Scenarios**:
- Encode 8-bit H.264 source → 8-bit AV1 QSV
- Encode 10-bit HEVC source → 10-bit AV1 QSV
- Encode HDR content → 10-bit AV1 QSV
- Verify Russian track removal still works
- Verify subtitle copying still works
- Verify chapter preservation still works

## Implementation Notes

### Docker Image Update

The new Docker image `lscr.io/linuxserver/ffmpeg:version-8.0-cli` includes:
- FFmpeg 8.0 with Intel VPL (Video Processing Library) support
- Intel media driver with QSV support
- Proper AV1 QSV codec implementation

### Environment Variable Requirement

`LIBVA_DRIVER_NAME=iHD` is required to ensure the Intel iHD driver is used instead of the older i965 driver. The iHD driver provides better support for Intel Arc GPUs.

### Profile Handling Difference

**VAAPI Profile Mapping**:
- Profile 0 (Main): 8-bit 4:2:0
- Profile 1 (High): 10-bit 4:2:0

**QSV Profile Mapping**:
- Profile 0 (Main): Both 8-bit and 10-bit 4:2:0
- Profile is determined by pixel format, not profile number

This means for QSV, we always use profile 0 (or "main") and let the pixel format (nv12 vs p010le) determine the bit depth.

### Filter Chain Simplification

QSV does not require the `extra_hw_frames=64` parameter that VAAPI needed. The simplified filter chain is:
- 8-bit: `format=nv12,hwupload`
- 10-bit: `format=p010le,hwupload`

### Backward Compatibility

Existing configuration files will continue to work:
- If `docker_image` is not updated, the system will use the configured image
- If `docker_image` is set to the old VAAPI image, encoding will fail for 10-bit sources
- Users should update their config to use the new image for 10-bit support

### Migration Path

1. Update `docker_image` in configuration to `lscr.io/linuxserver/ffmpeg:version-8.0-cli`
2. Deploy updated daemon binary
3. Restart daemon
4. Existing 8-bit encoding jobs will work with QSV
5. New 10-bit encoding jobs will now succeed

No data migration or job state changes are required.
