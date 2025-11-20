# Quick Reference - AV1 Quality Improvements

## TL;DR

**Problem**: All output is 8-bit, quality parameters are wrong
**Solution**: Detect source bit depth, use correct encoder parameters

## Key Changes at a Glance

### 1. Bit Depth Detection
```rust
// Add to FFProbeStream
pix_fmt: Option<String>
bits_per_raw_sample: Option<String>

// Detect bit depth
pub fn detect_bit_depth(&self) -> BitDepth {
    if bits_per_raw_sample == "10" { BitDepth::Bit10 }
    else if pix_fmt.contains("10") { BitDepth::Bit10 }
    else { BitDepth::Bit8 }
}
```

### 2. Dynamic Pixel Format
```rust
// OLD: Hardcoded 8-bit
filter_parts.push("format=nv12".to_string());

// NEW: Dynamic based on source
let format = match bit_depth {
    BitDepth::Bit8 => "nv12",
    BitDepth::Bit10 => "p010le",
};
filter_parts.push(format!("format={}", format));
```

### 3. Correct Quality Parameters
```rust
// OLD: Wrong parameter
ffmpeg_args.push("-quality".to_string());
ffmpeg_args.push("25".to_string());

// NEW: Correct parameters
ffmpeg_args.push("-rc_mode".to_string());
ffmpeg_args.push("CQP".to_string());
ffmpeg_args.push("-qp".to_string());
ffmpeg_args.push("28".to_string());
```

### 4. AV1 Profile
```rust
// NEW: Set profile based on bit depth
let profile = match bit_depth {
    BitDepth::Bit8 => 0,   // Main
    BitDepth::Bit10 => 1,  // High
};
ffmpeg_args.push("-profile:v".to_string());
ffmpeg_args.push(profile.to_string());
```

### 5. Fixed Quality Calculation
```rust
// OLD: Backwards logic
match codec {
    "hevc" => quality += 1,  // WRONG!
}

// NEW: Correct logic
match codec {
    "h264" => qp += 2,  // More compression (correct)
    "hevc" => qp -= 1,  // Less compression (correct)
}
```

### 6. Encoder Optimizations
```rust
// NEW: Add tiles for performance
ffmpeg_args.push("-tile_rows".to_string());
ffmpeg_args.push("1".to_string());
ffmpeg_args.push("-tile_cols".to_string());
ffmpeg_args.push("2".to_string());

// NEW: Set tier
ffmpeg_args.push("-tier:v".to_string());
ffmpeg_args.push("0".to_string());
```

## QP Value Guide

| Content | 8-bit QP | 10-bit QP | Expected Reduction |
|---------|----------|-----------|-------------------|
| 4K | 28-30 | 26-28 | 50-60% |
| 1080p | 30-32 | 28-30 | 60-70% |
| 720p | 32-34 | 30-32 | 65-75% |

**Adjustments**:
- H.264 source: +2 (can compress more)
- HEVC source: -1 (preserve quality)
- High bitrate (>0.6 bpppf): +3
- Low bitrate (<0.1 bpppf): -1
- High FPS (>50): -1

## Validation Commands

### Check bit depth
```bash
ffprobe -v error -select_streams v:0 \
  -show_entries stream=bits_per_raw_sample,pix_fmt,profile \
  -of default=noprint_wrappers=1 output.mkv
```

### Check HDR
```bash
ffprobe -v error -select_streams v:0 \
  -show_entries stream=color_transfer,color_primaries \
  -of default=noprint_wrappers=1 output.mkv
```

## Files to Modify

1. **`crates/daemon/src/ffprobe.rs`** (~45 min)
   - Add bit depth fields
   - Add detection logic

2. **`crates/daemon/src/job.rs`** (~15 min)
   - Add tracking fields

3. **`crates/daemon/src/ffmpeg_docker.rs`** (~90 min)
   - Add EncodingParams struct
   - Fix quality calculation
   - Update encoding function

4. **`crates/cli-daemon/src/main.rs`** (~30 min)
   - Extract bit depth
   - Pass to encoding

## Testing Checklist

- [ ] 8-bit source → 8-bit output ✓
- [ ] 10-bit source → 10-bit output ✓
- [ ] HDR preserved ✓
- [ ] QP in range 20-40 ✓
- [ ] File sizes reasonable ✓
- [ ] No quality loss ✓

## Common Mistakes to Avoid

❌ Using `quality` instead of `qp`
❌ Using `nv12` for 10-bit content
❌ Not setting `-profile:v` for 10-bit
❌ Backwards codec adjustment logic
❌ Forgetting `-rc_mode CQP`

## Expected Results

### Before
- All output: 8-bit
- 10-bit sources: Quality loss, banding
- HDR: Broken
- File sizes: Inconsistent

### After
- 8-bit → 8-bit: ✓
- 10-bit → 10-bit: ✓
- HDR: Preserved ✓
- File sizes: Predictable, optimal

## One-Line Summary

**Detect source bit depth, use p010le for 10-bit, use -qp with -rc_mode CQP, fix quality calculation logic.**

---

For detailed information, see:
- `IMPLEMENTATION_PLAN.md` - Step-by-step guide
- `AV1_QUALITY_IMPROVEMENT_SPEC.md` - Full specification
- `TECHNICAL_REFERENCE.md` - Parameter details
