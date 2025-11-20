# AV1 Quality Improvement - Implementation Plan

## Overview

This document provides a step-by-step implementation plan for improving AV1 encoding quality with proper bit depth preservation and optimized quality parameters.

## Pre-Implementation Checklist

- [ ] Review current codebase understanding
- [ ] Backup current working state
- [ ] Identify all files that need modification
- [ ] Understand Intel VAAPI AV1 encoder capabilities
- [ ] Review ffprobe output format for bit depth fields

## Implementation Steps

### Step 1: Extend FFProbeStream Structure ⏱️ 15 min

**File**: `crates/daemon/src/ffprobe.rs`

**Changes**:
1. Add new fields to `FFProbeStream`:
   - `pix_fmt: Option<String>` - Pixel format (e.g., "yuv420p10le")
   - `bits_per_raw_sample: Option<String>` - Bit depth as string
   - `color_space: Option<String>` - Color space (e.g., "bt2020nc")
   - `color_transfer: Option<String>` - Transfer characteristics (e.g., "smpte2084")
   - `color_primaries: Option<String>` - Color primaries (e.g., "bt2020")

2. Add helper function `detect_bit_depth(&self) -> BitDepth`

**Validation**:
- Compile check: `cargo check`
- Test with sample file to verify fields are populated

---

### Step 2: Create BitDepth Type and Detection Logic ⏱️ 30 min

**File**: `crates/daemon/src/ffprobe.rs`

**Changes**:
1. Add `BitDepth` enum:
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitDepth {
    Bit8,
    Bit10,
    Unknown,
}
```

2. Implement detection logic:
   - Parse `bits_per_raw_sample` (most reliable)
   - Parse `pix_fmt` for 10-bit indicators
   - Check HDR metadata (color_transfer = "smpte2084" or "arib-std-b67")
   - Default to 8-bit if unknown

3. Add helper function `is_hdr_content(&self) -> bool`

**Validation**:
- Unit tests for bit depth detection
- Test with 8-bit and 10-bit sample files

---

### Step 3: Extend Job Structure ⏱️ 15 min

**File**: `crates/daemon/src/job.rs`

**Changes**:
1. Add new fields to `Job` struct:
   - `source_bit_depth: Option<u8>` - 8 or 10
   - `source_pix_fmt: Option<String>` - Original pixel format
   - `target_bit_depth: Option<u8>` - 8 or 10
   - `av1_profile: Option<u8>` - 0 (Main) or 1 (High)
   - `is_hdr: Option<bool>` - HDR content flag

**Validation**:
- Compile check: `cargo check`
- Verify JSON serialization works

---

### Step 4: Create Encoding Parameters Structure ⏱️ 20 min

**File**: `crates/daemon/src/ffmpeg_docker.rs`

**Changes**:
1. Create new struct for encoding parameters:
```rust
pub struct EncodingParams {
    pub bit_depth: BitDepth,
    pub pixel_format: String,
    pub av1_profile: u8,
    pub qp: i32,
    pub is_hdr: bool,
}
```

2. Add function to determine encoding parameters:
```rust
pub fn determine_encoding_params(
    meta: &FFProbeData,
    input_file: &Path,
) -> EncodingParams
```

**Validation**:
- Compile check
- Logic review

---

### Step 5: Update Quality Calculation Function ⏱️ 45 min

**File**: `crates/daemon/src/ffmpeg_docker.rs`

**Changes**:
1. Rename `calculate_optimal_quality` to `calculate_optimal_qp`
2. Update signature to accept `bit_depth: BitDepth`
3. Change return type to `i32` (QP value, not "quality")
4. Update base QP calculation:
   - 4K: 26-28 (10-bit: 26, 8-bit: 28)
   - 1440p: 28-30
   - 1080p: 28-32
   - Lower: 30-34

5. **FIX codec adjustment logic** (CRITICAL):
   - H.264: `qp += 2` (more compression, CORRECT)
   - HEVC: `qp -= 1` (less compression, preserve quality, CORRECT)
   - VP9: `qp -= 1`
   - AV1: `qp += 0`

6. Update bitrate efficiency thresholds:
   - Very high (>0.6 bpppf): `qp += 3`
   - High (0.4-0.6): `qp += 2`
   - Medium (0.2-0.4): `qp += 1`
   - Low (0.1-0.2): `qp += 0`
   - Very low (<0.1): `qp -= 1`

7. Add bit depth adjustment:
   - 10-bit: `qp -= 1` (preserve extra color info)

8. Update clamping range: `qp.max(20).min(40)`

9. Update all comments and logging

**Validation**:
- Test with various source types
- Verify QP values are in expected range
- Check logs for clarity

---

### Step 6: Update Encoding Function ⏱️ 60 min

**File**: `crates/daemon/src/ffmpeg_docker.rs`

**Changes**:
1. Update `run_av1_vaapi_job` signature:
   - Add `encoding_params: &EncodingParams` parameter

2. Update filter chain for dynamic pixel format:
```rust
// Convert to appropriate format based on bit depth
filter_parts.push(format!("format={}", encoding_params.pixel_format));
```

3. **REPLACE** `-quality` parameter with proper rate control:
```rust
// Use Constant QP mode for quality-based encoding
ffmpeg_args.push("-rc_mode".to_string());
ffmpeg_args.push("CQP".to_string());

ffmpeg_args.push("-qp".to_string());
ffmpeg_args.push(encoding_params.qp.to_string());
```

4. Add AV1 profile specification:
```rust
ffmpeg_args.push("-profile:v".to_string());
ffmpeg_args.push(encoding_params.av1_profile.to_string());
```

5. Add tier specification:
```rust
ffmpeg_args.push("-tier:v".to_string());
ffmpeg_args.push("0".to_string()); // Main tier
```

6. Add tile configuration for better parallelization:
```rust
// Tile configuration for encoding efficiency
// 1 row, 2 columns is good for most content
ffmpeg_args.push("-tile_rows".to_string());
ffmpeg_args.push("1".to_string());
ffmpeg_args.push("-tile_cols".to_string());
ffmpeg_args.push("2".to_string());
```

7. Update logging to include bit depth info

8. Update `FFmpegResult` to include encoding params used

**Validation**:
- Test encoding with 8-bit source
- Test encoding with 10-bit source
- Verify output bit depth matches source
- Check file sizes are reasonable

---

### Step 7: Update Main Daemon Workflow ⏱️ 30 min

**File**: `crates/cli-daemon/src/main.rs`

**Changes**:
1. After probing file, detect bit depth:
```rust
let video_stream = meta.streams.iter()
    .find(|s| s.codec_type.as_deref() == Some("video"));

if let Some(stream) = video_stream {
    let bit_depth = stream.detect_bit_depth();
    let is_hdr = stream.is_hdr_content();
    
    job.source_bit_depth = Some(match bit_depth {
        BitDepth::Bit8 => 8,
        BitDepth::Bit10 => 10,
        BitDepth::Unknown => 8,
    });
    job.is_hdr = Some(is_hdr);
    // ... store other metadata
}
```

2. Determine encoding parameters:
```rust
let encoding_params = ffmpeg_docker::determine_encoding_params(&meta, &job.source_path);

job.target_bit_depth = Some(match encoding_params.bit_depth {
    BitDepth::Bit8 => 8,
    BitDepth::Bit10 => 10,
    BitDepth::Unknown => 8,
});
job.av1_profile = Some(encoding_params.av1_profile);
```

3. Update logging:
```rust
info!("Job {}: Source: {}x{}, {}-bit, {}, {:.2} fps, {} Mbps",
    job.id, width, height, 
    job.source_bit_depth.unwrap_or(8),
    source_codec,
    fps,
    bitrate_mbps
);

info!("Job {}: Target: {}-bit AV1 (profile {}), QP: {}",
    job.id,
    job.target_bit_depth.unwrap_or(8),
    job.av1_profile.unwrap_or(0),
    encoding_params.qp
);
```

4. Pass encoding params to encoding function:
```rust
let ffmpeg_result = ffmpeg_docker::run_av1_vaapi_job(
    cfg,
    &job.source_path,
    &temp_output,
    &meta,
    &decision,
    &encoding_params,  // NEW
).await?;
```

**Validation**:
- Full workflow test with 8-bit file
- Full workflow test with 10-bit file
- Verify job JSON contains all new fields
- Check logs are informative

---

### Step 8: Update Expected Reduction Calculation ⏱️ 15 min

**File**: `crates/daemon/src/ffmpeg_docker.rs`

**Changes**:
1. Update `calculate_expected_reduction` function:
   - Accept `bit_depth` parameter
   - Adjust expectations for 10-bit (slightly less compression)
   - Update QP-based reduction estimates

2. Update reduction estimates:
   - QP 20-24: 40-50% reduction
   - QP 25-28: 50-60% reduction
   - QP 29-32: 60-70% reduction
   - QP 33-36: 70-75% reduction
   - QP 37-40: 75-80% reduction

3. Adjust for bit depth:
   - 10-bit: Reduce expected compression by 5%

**Validation**:
- Review estimates against actual results
- Update if needed based on testing

---

### Step 9: Testing & Validation ⏱️ 90 min

**Test Cases**:

1. **8-bit H.264 1080p** (common web source)
   - Expected: 8-bit AV1, QP ~30-32, 60-70% reduction
   - Verify: No banding, good quality

2. **10-bit HEVC 4K** (high quality source)
   - Expected: 10-bit AV1, QP ~26-28, 45-55% reduction
   - Verify: Bit depth preserved, no quality loss

3. **10-bit HDR content**
   - Expected: 10-bit AV1, HDR metadata preserved
   - Verify: HDR flags in output, proper color

4. **8-bit low bitrate source**
   - Expected: 8-bit AV1, QP ~28-30, 40-50% reduction
   - Verify: No over-compression artifacts

5. **Various codecs** (H.264, HEVC, VP9)
   - Verify: Codec-specific adjustments work correctly

**Validation Steps**:
1. Run encoding on test files
2. Check output with ffprobe:
   ```bash
   ffprobe -v error -select_streams v:0 \
     -show_entries stream=pix_fmt,bits_per_raw_sample,profile \
     -of json output.mkv
   ```
3. Visual inspection for quality
4. Verify file size reductions
5. Check job JSON for correct metadata

---

### Step 10: Documentation Updates ⏱️ 20 min

**Files to Update**:
1. `README.md`: Add section on bit depth preservation
2. Code comments: Ensure all new functions are documented
3. This implementation plan: Mark as complete

**Documentation Content**:
- Explain bit depth detection
- Document QP parameter ranges
- Explain quality calculation logic
- Add examples of expected results

---

## Post-Implementation Checklist

- [ ] All files compile without errors
- [ ] All test cases pass
- [ ] 8-bit sources produce 8-bit output
- [ ] 10-bit sources produce 10-bit output
- [ ] HDR metadata preserved
- [ ] QP values in expected range (20-40)
- [ ] File size reductions meet expectations
- [ ] Logs are clear and informative
- [ ] Job JSON contains all new fields
- [ ] Documentation updated
- [ ] Code reviewed for quality

## Rollback Plan

If issues arise:
1. Git revert to previous commit
2. Keep spec documents for future attempt
3. Document what went wrong
4. Adjust plan and retry

## Success Metrics

After implementation, verify:
1. ✅ 100% of 8-bit sources → 8-bit output
2. ✅ 100% of 10-bit sources → 10-bit output
3. ✅ HDR content properly flagged and preserved
4. ✅ Average file size reduction: 50-65%
5. ✅ No visual quality degradation
6. ✅ No encoding failures due to parameter issues

## Timeline Summary

| Step | Duration | Cumulative |
|------|----------|------------|
| 1. FFProbeStream | 15 min | 15 min |
| 2. BitDepth Detection | 30 min | 45 min |
| 3. Job Structure | 15 min | 60 min |
| 4. Encoding Params | 20 min | 80 min |
| 5. Quality Calculation | 45 min | 125 min |
| 6. Encoding Function | 60 min | 185 min |
| 7. Main Daemon | 30 min | 215 min |
| 8. Reduction Calc | 15 min | 230 min |
| 9. Testing | 90 min | 320 min |
| 10. Documentation | 20 min | 340 min |

**Total: ~5.5 hours**

## Notes

- Take breaks between major steps
- Commit after each successful step
- Test incrementally, don't wait until the end
- If stuck, refer back to spec document
- Keep detailed notes of any issues encountered
