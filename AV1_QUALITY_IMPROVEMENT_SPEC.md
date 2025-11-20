# AV1 Quality Improvement Specification

## Executive Summary

This specification outlines improvements to the AV1 encoding pipeline to:
1. **Preserve source bit depth** (8-bit → 8-bit, 10-bit → 10-bit)
2. **Fix quality parameter semantics** for Intel VAAPI AV1 encoder
3. **Improve quality calculation algorithm** for optimal compression/quality balance
4. **Add proper AV1 encoder parameters** (profile, tiles, rate control)

## Current Issues

### 🔴 Critical Issues

1. **Hardcoded 8-bit encoding**: All output is 8-bit (NV12) regardless of source
2. **Incorrect quality parameter usage**: Using `-quality` which is speed/quality tradeoff, not bitrate/quality
3. **Missing bit depth detection**: No code to detect source bit depth
4. **No AV1 profile specification**: Not setting profile for 10-bit support

### 🟡 Medium Issues

1. **Quality calculation logic errors**: Codec adjustment signs are backwards
2. **Narrow quality range**: 20-30 may not be optimal for VAAPI
3. **Missing encoder optimizations**: No tile configuration, no rate control mode

## Proposed Solution

### Phase 1: Bit Depth Detection & Preservation

#### 1.1 Extend FFProbeStream Structure

Add fields to capture bit depth information:
- `pix_fmt` (pixel format, e.g., "yuv420p", "yuv420p10le")
- `bits_per_raw_sample` (actual bit depth)
- `color_space` (for HDR detection)
- `color_transfer` (for HDR detection)
- `color_primaries` (for HDR detection)

#### 1.2 Create BitDepth Enum

```rust
pub enum BitDepth {
    Bit8,
    Bit10,
    Unknown,
}
```

#### 1.3 Implement Bit Depth Detection Function

Parse pixel format and bits_per_raw_sample to determine source bit depth.

### Phase 2: Encoding Parameter Improvements

#### 2.1 Dynamic Pixel Format Selection

- 8-bit sources → `format=nv12` → AV1 Profile 0 (Main)
- 10-bit sources → `format=p010le` → AV1 Profile 1 (High)

#### 2.2 Replace `-quality` with `-qp` (Quantization Parameter)

Intel VAAPI AV1 encoder quality control:
- `-qp <value>`: Constant Quantization Parameter (0-255, lower = better quality)
- `-rc_mode CQP`: Constant Quality mode
- Range: 20-40 for practical use (not 20-30)

#### 2.3 Add AV1-Specific Parameters

- `-profile:v`: 0 (Main/8-bit) or 1 (High/10-bit)
- `-tier:v`: 0 (Main tier, sufficient for most content)
- `-tile_rows` / `-tile_cols`: Optimize for parallelization
- `-rc_mode`: CQP for constant quality

### Phase 3: Quality Calculation Algorithm Improvements

#### 3.1 Fix Codec Adjustment Logic

Current logic is inverted. Correct approach:
- H.264 sources: Can compress more aggressively (LOWER qp = higher quality, but we want compression, so HIGHER qp)
- HEVC/VP9 sources: Already efficient, preserve quality (LOWER qp)

#### 3.2 Expand Quality Range

- 8-bit content: QP 24-38 (wider range for more flexibility)
- 10-bit content: QP 22-36 (slightly higher quality to preserve bit depth benefits)

#### 3.3 Add Bit Depth Consideration

10-bit content should use slightly lower QP values to preserve the additional color information.

#### 3.4 Improve Bitrate Efficiency Calculation

Refine bpppf (bits per pixel per frame) thresholds:
- Very high bitrate (>0.6 bpppf): QP +3 (aggressive compression)
- High bitrate (0.4-0.6 bpppf): QP +2
- Medium bitrate (0.2-0.4 bpppf): QP +1
- Low bitrate (0.1-0.2 bpppf): QP +0 (baseline)
- Very low bitrate (<0.1 bpppf): QP -1 (preserve quality)

### Phase 4: Enhanced Metadata Tracking

#### 4.1 Extend Job Structure

Add fields to track:
- `source_bit_depth`: 8 or 10
- `source_pix_fmt`: Original pixel format
- `target_bit_depth`: 8 or 10
- `av1_profile`: 0 or 1
- `is_hdr`: Boolean for HDR content

#### 4.2 Update Logging

Add bit depth information to all quality calculation logs.

## Implementation Plan

### Step 1: Update Data Structures (30 min)

Files to modify:
- `crates/daemon/src/ffprobe.rs`: Add pix_fmt, bits_per_raw_sample, color_* fields
- `crates/daemon/src/job.rs`: Add bit depth tracking fields

### Step 2: Implement Bit Depth Detection (45 min)

Files to modify:
- `crates/daemon/src/ffprobe.rs`: Add `detect_bit_depth()` function
- Add helper functions for parsing pixel formats

### Step 3: Update Encoding Pipeline (60 min)

Files to modify:
- `crates/daemon/src/ffmpeg_docker.rs`:
  - Update `run_av1_vaapi_job()` to accept bit depth parameter
  - Implement dynamic pixel format selection
  - Replace `-quality` with `-qp` and `-rc_mode CQP`
  - Add AV1 profile, tier, and tile parameters

### Step 4: Improve Quality Calculation (45 min)

Files to modify:
- `crates/daemon/src/ffmpeg_docker.rs`:
  - Update `calculate_optimal_quality()` to accept bit depth
  - Fix codec adjustment logic (reverse signs)
  - Expand quality range (20-40)
  - Add bit depth consideration
  - Refine bitrate efficiency thresholds

### Step 5: Update Main Daemon (30 min)

Files to modify:
- `crates/cli-daemon/src/main.rs`:
  - Extract bit depth from metadata
  - Pass bit depth to quality calculation
  - Pass bit depth to encoding function
  - Update job tracking with bit depth info

### Step 6: Testing & Validation (60 min)

Test cases:
1. 8-bit H.264 1080p source → 8-bit AV1
2. 10-bit HEVC 4K source → 10-bit AV1
3. 10-bit HDR source → 10-bit AV1 with HDR preservation
4. Various bitrate sources to validate quality calculation

## Expected Outcomes

### Quality Improvements

1. **Bit depth preservation**: No more 10-bit → 8-bit quality loss
2. **Better compression ratios**: Proper QP usage should achieve 55-75% size reduction
3. **Optimal quality settings**: Source-aware quality calculation
4. **HDR support**: Proper handling of HDR content

### File Size Expectations

- 8-bit H.264 high bitrate → 60-70% reduction
- 8-bit H.264 medium bitrate → 50-60% reduction
- 10-bit HEVC high bitrate → 45-55% reduction
- 10-bit HEVC medium bitrate → 35-45% reduction

### Performance

- No significant performance impact (hardware encoding)
- Slightly larger files for 10-bit (expected, preserves quality)

## Risk Assessment

### Low Risk
- Bit depth detection (straightforward parsing)
- Data structure updates (additive changes)

### Medium Risk
- Quality calculation changes (needs testing to validate)
- QP parameter range (may need tuning per GPU)

### Mitigation
- Extensive logging for debugging
- Gradual rollout with test files
- Keep old quality calculation as fallback option

## Success Criteria

1. ✅ 8-bit sources produce 8-bit AV1 output
2. ✅ 10-bit sources produce 10-bit AV1 output
3. ✅ HDR metadata preserved in 10-bit output
4. ✅ Quality calculation produces appropriate QP values (20-40 range)
5. ✅ File size reductions meet expectations (35-75% depending on source)
6. ✅ Visual quality inspection shows no banding or artifacts
7. ✅ All metadata properly tracked in job files

## Technical Reference

### Intel VAAPI AV1 Encoder Parameters

- **-qp**: Quantization Parameter (0-255, lower = better quality)
  - Practical range: 20-40
  - Recommended: 24-32 for most content
  
- **-rc_mode**: Rate control mode
  - CQP: Constant Quantization Parameter (quality-based)
  - CBR: Constant Bitrate
  - VBR: Variable Bitrate
  
- **-profile:v**: AV1 profile
  - 0: Main (8-bit, 4:2:0)
  - 1: High (10-bit, 4:2:0)
  - 2: Professional (12-bit, 4:2:2/4:4:4)
  
- **-tier:v**: AV1 tier
  - 0: Main tier (sufficient for most content)
  - 1: High tier (for very high bitrate content)

### Pixel Format Reference

- **8-bit formats**:
  - nv12: 4:2:0, 8-bit (most common)
  - yuv420p: 4:2:0, 8-bit (planar)
  
- **10-bit formats**:
  - p010le: 4:2:0, 10-bit (VAAPI preferred)
  - yuv420p10le: 4:2:0, 10-bit (planar)

### Bit Depth Detection Logic

1. Check `bits_per_raw_sample` field (most reliable)
2. Parse `pix_fmt` for "10" suffix (e.g., "yuv420p10le")
3. Check for HDR metadata (implies 10-bit)
4. Default to 8-bit if unknown

## Timeline

- **Total estimated time**: 4-5 hours
- **Testing time**: 1-2 hours
- **Documentation**: 30 minutes

## Appendix: Code Examples

### Example: Bit Depth Detection

```rust
pub fn detect_bit_depth(stream: &FFProbeStream) -> BitDepth {
    // Method 1: Check bits_per_raw_sample
    if let Some(bits) = stream.bits_per_raw_sample {
        if bits >= 10 {
            return BitDepth::Bit10;
        } else if bits == 8 {
            return BitDepth::Bit8;
        }
    }
    
    // Method 2: Parse pixel format
    if let Some(ref pix_fmt) = stream.pix_fmt {
        if pix_fmt.contains("10") || pix_fmt.contains("p010") {
            return BitDepth::Bit10;
        }
    }
    
    // Method 3: Check for HDR (implies 10-bit)
    if is_hdr_content(stream) {
        return BitDepth::Bit10;
    }
    
    // Default to 8-bit
    BitDepth::Bit8
}
```

### Example: Dynamic Encoding Parameters

```rust
let (pixel_format, av1_profile) = match bit_depth {
    BitDepth::Bit8 => ("nv12", 0),
    BitDepth::Bit10 => ("p010le", 1),
    BitDepth::Unknown => ("nv12", 0), // Safe default
};

// Add to filter chain
filter_parts.push(format!("format={}", pixel_format));

// Add to encoder params
ffmpeg_args.push("-profile:v".to_string());
ffmpeg_args.push(av1_profile.to_string());
```

### Example: Improved Quality Calculation

```rust
// Base quality from resolution and bit depth
let mut qp = if height >= 2160 {
    if bit_depth == BitDepth::Bit10 { 26 } else { 28 }
} else if height >= 1080 {
    if bit_depth == BitDepth::Bit10 { 28 } else { 30 }
} else {
    if bit_depth == BitDepth::Bit10 { 30 } else { 32 }
};

// Adjust for source codec (CORRECTED LOGIC)
match source_codec.as_str() {
    "h264" | "avc" => {
        // H.264 is inefficient, can compress more
        qp += 2; // Higher QP = more compression
    }
    "hevc" | "h265" => {
        // HEVC already efficient, preserve quality
        qp -= 1; // Lower QP = less compression
    }
    _ => {}
}

// Clamp to valid range
qp = qp.max(20).min(40);
```
