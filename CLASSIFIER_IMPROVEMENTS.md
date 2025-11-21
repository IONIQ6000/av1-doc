# Source Classification & Validation Improvements

## Overview

Enhanced the video transcoding system with robust source classification, output validation, and comprehensive logging to prevent corruption of web downloads and ensure reliable encoding.

## 1. Enhanced Source Classifier

### Previous Implementation
- **4 detection signals**: Filename tokens, muxing app, VFR detection, odd dimensions
- **Simple scoring**: Basic threshold (0.3 for web, -0.2 for disc)
- **Limited accuracy**: Could misclassify edge cases

### New Implementation
- **7 detection signals** with weighted scoring:

#### Signal 1: Filename Analysis (Weight: 0.35)
- Web tokens: WEB-DL, WEBRIP, NF, AMZN, HULU, DSNP, ATVP
- Disc tokens: BLURAY, BDRIP, REMUX, BDMV, DVD

#### Signal 2: Container Format (Weight: 0.15)
- MP4/MOV containers → Web indicator
- Matroska with specific tags → Context-dependent

#### Signal 3: Muxing Tools (Weight: 0.1-0.15)
- Web tools: mkvmerge, handbrake
- Disc tools: MakeMKV, AnyDVD

#### Signal 4: Audio Codec Analysis (Weight: 0.1-0.15)
- Web codecs: AAC, Opus, MP3
- Disc codecs: TrueHD, DTS, FLAC, PCM
- Multiple E-AC3 tracks → Disc indicator

#### Signal 5: Stream Count Patterns (Weight: 0.1-0.15)
- Single audio + few subs → Web
- Multiple audio (3+) or many subs (5+) → Disc

#### Signal 6: Video Stream Analysis (Weight: 0.2)
- Variable frame rate → Web (strong indicator)
- Odd dimensions → Web
- Encoder tags (x264 settings) → Web

#### Signal 7: Bitrate Efficiency (Weight: 0.1)
- Low bitrate/pixel (<0.15) → Web
- High bitrate/pixel (>0.3) → Disc

### Improved Thresholds
- **WebLike**: Score ≥ 0.4 (increased from 0.3)
- **DiscLike**: Score ≤ -0.3 (increased from -0.2)
- **Unknown**: Between thresholds (conservative handling)

### Impact on Encoding

**Web Sources** get special flags to prevent corruption:
```bash
-fflags +genpts          # Generate presentation timestamps
-copyts                  # Copy timestamps (preserve timing)
-start_at_zero          # Normalize start time
-vsync 0                # Passthrough timestamps (no frame dropping)
-avoid_negative_ts make_zero  # Fix negative timestamp issues
```

**Disc Sources** use standard encoding (no special flags needed).

## 2. Output Validation System

### New Validation Function: `validate_output()`

Performs 10 comprehensive checks on encoded output:

#### Check 1: File Existence & Size
- Verifies file exists
- Checks file is not empty (>0 bytes)
- Warns if file is suspiciously small (<1MB)

#### Check 2: FFprobe Validation
- Runs ffprobe on output
- Failure indicates corrupted/unreadable file
- Catches container-level corruption

#### Check 3: Video Stream Verification
- Ensures video stream exists
- Validates stream is readable

#### Check 4: Codec Verification
- Confirms codec is AV1
- Detects encoding failures that produce wrong codec

#### Check 5: Bit Depth Validation
- Verifies output bit depth matches expected
- 8-bit source → 8-bit output (yuv420p)
- 10-bit source → 10-bit output (yuv420p10le)

#### Check 6: Pixel Format Validation
- Checks pixel format matches bit depth
- Detects format conversion errors

#### Check 7: Dimension Validation
- Verifies dimensions are valid (>0)
- Warns on odd dimensions (playback issues)

#### Check 8: Frame Rate Analysis
- Detects VFR in output (corruption indicator)
- Checks for timestamp issues
- Warns if avg_frame_rate ≠ r_frame_rate

#### Check 9: Audio Stream Preservation
- Verifies audio streams were copied
- Warns if no audio (may be intentional)

#### Check 10: Bitrate Sanity Check
- Detects abnormally low bitrates
- Catches encoding failures that produce tiny files

### Validation Result
```rust
pub struct ValidationResult {
    pub is_valid: bool,        // Overall pass/fail
    pub issues: Vec<String>,   // Critical problems (fail job)
    pub warnings: Vec<String>, // Non-critical issues (log only)
}
```

### Integration
- Runs automatically after encoding completes
- Before file replacement (catches corruption early)
- Failed validation → Job marked as Failed, temp file deleted
- Warnings logged but don't fail the job

## 3. Enhanced Logging

### Classification Logging

**Before:**
```
Job abc123: Source classification: WebLike (web_like: true)
```

**After:**
```
Job abc123: 🎯 Source classification: WebLike (score: 0.65, web_like: true)
Job abc123: 📋 Classification reasons:
Job abc123:    - filename contains WEB-DL
Job abc123:    - web audio codec: aac
Job abc123:    - variable frame rate detected
Job abc123:    - minimal streams: 1 audio, 2 subs (web pattern)
Job abc123: 🌐 Using WEB encoding strategy (VFR handling, timestamp fixes)
```

### Validation Logging

**Success:**
```
Job abc123: 🔍 Validating output file: /path/to/output.mkv
Job abc123: ✅ Output validation passed with no warnings
```

**With Warnings:**
```
Job abc123: 🔍 Validating output file: /path/to/output.mkv
Job abc123: ⚠️  Output validation warnings: 2
Job abc123:    - Output bit depth (Bit8) differs from expected (Bit10)
Job abc123:    - Variable frame rate detected in output (avg: 29.97, r: 30.00)
```

**Failure:**
```
Job abc123: 🔍 Validating output file: /path/to/output.mkv
Job abc123: ❌ Output validation failed: 2 issues, 1 warnings
Job abc123: ❌ Validation issue: Output codec is 'h264', expected 'av1'
Job abc123: ❌ Validation issue: No video stream found in output
Job abc123: ❌ output validation failed: Output codec is 'h264', expected 'av1', No video stream found in output
```

### Encoding Parameter Logging

Enhanced to show full decision chain:
```
Job abc123: 🎬 Encoding params (QSV): 10-bit (profile 0), QP 28, format p010le, HDR: true
Job abc123: Encoding plan - Source: 10-bit HDR → Target: 10-bit AV1 (profile 0), QP: 28
```

## Benefits

### 1. Prevents Web Download Corruption
- Accurate detection of web sources
- Proper VFR/timestamp handling
- Reduced false negatives

### 2. Early Corruption Detection
- Validates output before file replacement
- Catches encoding failures immediately
- Prevents corrupted files from replacing originals

### 3. Improved Debugging
- Detailed classification reasoning
- Validation issue tracking
- Clear encoding strategy logging

### 4. Confidence in Results
- Multiple validation checks
- Comprehensive error reporting
- Warnings for edge cases

## Testing

All existing tests pass:
```
running 13 tests
test ffmpeg_docker::tests::test_10bit_filter_chain ... ok
test ffmpeg_docker::tests::test_8bit_filter_chain ... ok
test ffmpeg_docker::tests::test_av1_qsv_codec_selection ... ok
test ffmpeg_docker::tests::test_device_path_in_initialization ... ok
test ffmpeg_docker::tests::test_filter_chain_ordering ... ok
test ffmpeg_docker::tests::test_global_quality_parameter ... ok
test ffmpeg_docker::tests::test_libva_driver_environment_variable ... ok
test ffmpeg_docker::tests::test_pixel_format_selection ... ok
test ffmpeg_docker::tests::test_qsv_hardware_initialization ... ok
test ffmpeg_docker::tests::test_qsv_hwupload_filter ... ok
test ffmpeg_docker::tests::test_qsv_profile_for_10bit ... ok
test ffmpeg_docker::tests::test_quality_calculation_preservation ... ok
test ffmpeg_docker::tests::test_quality_value_in_result ... ok

test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

## Files Modified

1. **crates/daemon/src/classifier.rs**
   - Enhanced `classify_web_source()` with 7 detection signals
   - Improved scoring thresholds
   - Added detailed reasoning

2. **crates/daemon/src/ffmpeg_docker.rs**
   - Added `ValidationResult` struct
   - Added `validate_output()` function with 10 checks
   - Exported validation for use in daemon

3. **crates/cli-daemon/src/main.rs**
   - Enhanced classification logging
   - Added validation call after encoding
   - Added encoding strategy logging
   - Improved error reporting

## Recommendations for Users

1. **Monitor logs** for classification accuracy
2. **Check validation warnings** for edge cases
3. **Report misclassifications** to improve detection
4. **Review Unknown classifications** - may need manual handling

## Future Enhancements

Potential improvements:
- Machine learning classifier (train on user's library)
- User feedback loop (correct misclassifications)
- Configurable validation strictness
- Validation report export (JSON/CSV)
- Classification confidence scores
