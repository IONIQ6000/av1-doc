# Implementation Summary - AV1 Quality Improvements

## Quick Overview

This implementation adds proper bit depth preservation and optimized quality parameters to the AV1 encoding pipeline.

## What's Being Fixed

### 🔴 Critical Fixes

1. **Bit Depth Preservation**
   - Current: All output is 8-bit (NV12)
   - Fixed: 8-bit sources → 8-bit output, 10-bit sources → 10-bit output

2. **Correct Quality Parameters**
   - Current: Using `-quality` (speed/quality tradeoff)
   - Fixed: Using `-qp` with `-rc_mode CQP` (quality/bitrate tradeoff)

3. **AV1 Profile Support**
   - Current: No profile specified (defaults to 8-bit)
   - Fixed: Profile 0 for 8-bit, Profile 1 for 10-bit

### 🟡 Quality Improvements

1. **Fixed Quality Calculation Logic**
   - Corrected codec adjustment signs (was backwards)
   - Expanded QP range from 20-30 to 20-40
   - Added bit depth consideration
   - Refined bitrate efficiency thresholds

2. **Added Encoder Optimizations**
   - Tile configuration for parallelization
   - Explicit rate control mode (CQP)
   - Tier specification

## Key Changes by File

### `crates/daemon/src/ffprobe.rs`
- ✅ Add pixel format, bit depth, and color metadata fields
- ✅ Add `BitDepth` enum (Bit8, Bit10, Unknown)
- ✅ Add `detect_bit_depth()` method
- ✅ Add `is_hdr_content()` helper

### `crates/daemon/src/job.rs`
- ✅ Add bit depth tracking fields
- ✅ Add HDR flag
- ✅ Add AV1 profile tracking

### `crates/daemon/src/ffmpeg_docker.rs`
- ✅ Create `EncodingParams` struct
- ✅ Add `determine_encoding_params()` function
- ✅ Rename `calculate_optimal_quality()` → `calculate_optimal_qp()`
- ✅ Fix quality calculation logic (reverse codec adjustments)
- ✅ Update `run_av1_vaapi_job()` for dynamic bit depth
- ✅ Replace `-quality` with `-qp` and `-rc_mode CQP`
- ✅ Add `-profile:v`, `-tier:v`, tile parameters

### `crates/cli-daemon/src/main.rs`
- ✅ Extract bit depth from metadata
- ✅ Determine encoding parameters
- ✅ Pass encoding params to encoding function
- ✅ Update job tracking with bit depth info
- ✅ Enhance logging

## Before vs After

### Before (Current State)

```rust
// Hardcoded 8-bit
filter_parts.push("format=nv12".to_string());

// Wrong parameter
ffmpeg_args.push("-quality".to_string());
ffmpeg_args.push("25".to_string());

// No profile
// No rate control mode
// No tiles
```

**Result**: All output is 8-bit, quality parameter doesn't work as expected

### After (Improved)

```rust
// Dynamic bit depth
let pixel_format = match bit_depth {
    BitDepth::Bit8 => "nv12",
    BitDepth::Bit10 => "p010le",
    _ => "nv12",
};
filter_parts.push(format!("format={}", pixel_format));

// Correct parameters
ffmpeg_args.push("-rc_mode".to_string());
ffmpeg_args.push("CQP".to_string());
ffmpeg_args.push("-qp".to_string());
ffmpeg_args.push("28".to_string());

// Profile for bit depth
ffmpeg_args.push("-profile:v".to_string());
ffmpeg_args.push("1".to_string()); // 10-bit

// Tiles for performance
ffmpeg_args.push("-tile_rows".to_string());
ffmpeg_args.push("1".to_string());
ffmpeg_args.push("-tile_cols".to_string());
ffmpeg_args.push("2".to_string());
```

**Result**: Proper bit depth preservation, correct quality control, optimized encoding

## Quality Calculation Changes

### Before

```rust
// Base quality
let mut quality = if height >= 2160 { 24 } else { 25 };

// WRONG: Adding to quality for HEVC (backwards)
match source_codec {
    "hevc" => quality += 1, // Should be -= 1
    _ => {}
}

// Narrow range
quality.max(20).min(30)
```

### After

```rust
// Base QP with bit depth consideration
let mut qp = if height >= 2160 {
    if bit_depth == BitDepth::Bit10 { 26 } else { 28 }
} else if height >= 1080 {
    if bit_depth == BitDepth::Bit10 { 28 } else { 30 }
} else {
    if bit_depth == BitDepth::Bit10 { 30 } else { 32 }
};

// CORRECT: Codec adjustments
match source_codec {
    "h264" => qp += 2,  // Can compress more
    "hevc" => qp -= 1,  // Preserve quality
    _ => {}
}

// Wider range
qp.max(20).min(40)
```

## Expected Results

### File Size Reductions

| Source Type | Before | After | Improvement |
|-------------|--------|-------|-------------|
| 8-bit H.264 high bitrate | ~60% | ~65% | Better compression |
| 10-bit HEVC | ~60% (quality loss!) | ~45% | Preserves quality |
| 10-bit HDR | ~60% (broken!) | ~45% | HDR preserved |

### Quality Improvements

1. **No more 10-bit → 8-bit degradation**
   - Banding eliminated
   - Color depth preserved
   - HDR works correctly

2. **Better compression for H.264 sources**
   - More aggressive QP for inefficient codecs
   - Smaller files without quality loss

3. **Better quality for HEVC sources**
   - Less aggressive QP for efficient codecs
   - Preserves already-good quality

## Testing Checklist

After implementation, test these scenarios:

- [ ] 8-bit H.264 1080p → Verify 8-bit AV1 output
- [ ] 10-bit HEVC 4K → Verify 10-bit AV1 output
- [ ] 10-bit HDR content → Verify HDR metadata preserved
- [ ] Low bitrate source → Verify no over-compression
- [ ] High bitrate source → Verify good compression
- [ ] Various codecs → Verify codec-specific adjustments work

## Validation Commands

### Check output bit depth
```bash
ffprobe -v error -select_streams v:0 \
  -show_entries stream=pix_fmt,bits_per_raw_sample,profile \
  -of json output.mkv
```

Expected for 10-bit:
```json
{
  "streams": [{
    "pix_fmt": "yuv420p10le",
    "bits_per_raw_sample": "10",
    "profile": "High"
  }]
}
```

### Check HDR metadata
```bash
ffprobe -v error -select_streams v:0 \
  -show_entries stream=color_space,color_transfer,color_primaries \
  -of json output.mkv
```

Expected for HDR:
```json
{
  "streams": [{
    "color_space": "bt2020nc",
    "color_transfer": "smpte2084",
    "color_primaries": "bt2020"
  }]
}
```

## Risk Mitigation

### Low Risk Items
- Data structure changes (additive only)
- Bit depth detection (straightforward parsing)
- Logging improvements

### Medium Risk Items
- Quality calculation changes (needs testing)
- Encoding parameter changes (may need tuning)

### Mitigation Strategy
1. Implement incrementally
2. Test after each major change
3. Keep detailed logs
4. Easy rollback with git
5. Test with diverse content

## Timeline

- **Implementation**: 3.5 hours
- **Testing**: 1.5 hours
- **Documentation**: 0.5 hours
- **Total**: ~5.5 hours

## Success Criteria

Implementation is successful when:

1. ✅ 8-bit sources produce 8-bit AV1 output
2. ✅ 10-bit sources produce 10-bit AV1 output
3. ✅ HDR metadata is preserved
4. ✅ QP values are in range 20-40
5. ✅ File sizes meet expectations (35-75% reduction)
6. ✅ No visual quality degradation
7. ✅ All metadata tracked in job files
8. ✅ Logs are clear and informative

## Next Steps

1. Review this summary and the detailed spec
2. Confirm approach is correct
3. Begin implementation following the step-by-step plan
4. Test incrementally
5. Validate results
6. Update documentation

## Questions to Consider

Before starting implementation:

1. **GPU Compatibility**: Does your Intel Arc GPU support 10-bit AV1 encoding?
   - Check: `vainfo | grep AV1`
   - Should show: `VAProfileAV1Profile0` and `VAProfileAV1Profile1`

2. **FFmpeg Version**: Does your Docker image support these parameters?
   - Check: `ffmpeg -h encoder=av1_vaapi`
   - Should show: `-qp`, `-rc_mode`, `-profile`

3. **Test Content**: Do you have test files for validation?
   - Need: 8-bit H.264, 10-bit HEVC, 10-bit HDR samples

4. **Backup Strategy**: Can you easily rollback if needed?
   - Ensure: Git commits are clean, can revert easily

## References

- `AV1_QUALITY_IMPROVEMENT_SPEC.md` - Detailed specification
- `IMPLEMENTATION_PLAN.md` - Step-by-step implementation guide
- `TECHNICAL_REFERENCE.md` - Technical details and parameters

---

**Ready to proceed?** Start with Step 1 in `IMPLEMENTATION_PLAN.md`
