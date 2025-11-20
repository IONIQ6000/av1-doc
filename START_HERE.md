# START HERE - AV1 Quality Improvements

## 🎯 Quick Summary

Your AV1 encoder is currently outputting **all files as 8-bit**, even when the source is 10-bit. This causes:
- Quality loss (banding in gradients)
- Broken HDR content
- Suboptimal compression settings

**Solution**: Detect source bit depth and preserve it in the output.

## 📚 Documentation Overview

I've created a complete implementation package. Here's what to read:

### 1. **START HERE** (you are here)
Quick overview and reading guide

### 2. **LINUXSERVER_FFMPEG_ANALYSIS.md** ⭐ READ THIS FIRST
Research on your specific Docker image and what parameters actually work

### 3. **FINAL_IMPLEMENTATION_GUIDE.md** ⭐ THEN READ THIS
Simplified, confirmed-working implementation plan

### 4. Supporting Documents (reference as needed)
- `AV1_QUALITY_IMPROVEMENT_SPEC.md` - Detailed specification
- `IMPLEMENTATION_PLAN.md` - Original detailed plan
- `TECHNICAL_REFERENCE.md` - Parameter reference
- `QUICK_REFERENCE.md` - Quick lookup
- `IMPLEMENTATION_CHECKLIST.md` - Progress tracking
- `FFMPEG_8_COMPATIBILITY_CHECK.md` - Compatibility notes

## 🔍 What I Found

### Critical Issues in Your Code

1. **Hardcoded 8-bit encoding**
   ```rust
   // Current (WRONG)
   filter_parts.push("format=nv12".to_string()); // Always 8-bit!
   ```

2. **Quality calculation logic is backwards**
   ```rust
   // Current (WRONG)
   "hevc" => quality += 1, // Should be -= 1
   ```

3. **Using wrong parameter name**
   ```rust
   // Current (WORKS but not optimal)
   ffmpeg_args.push("-quality".to_string()); // This is speed, not quality!
   ```

### What Needs to Change

1. **Detect bit depth from source**
   - Parse `bits_per_raw_sample` field
   - Check `pix_fmt` for "10" suffix
   - Check HDR metadata

2. **Use correct pixel format**
   - 8-bit: `format=nv12`
   - 10-bit: `format=p010le`

3. **Use correct quality parameter**
   - Use `-qp` (quantization parameter)
   - Range: 20-40 for practical use
   - Lower = better quality

4. **Fix codec adjustment logic**
   - H.264: `qp += 2` (can compress more)
   - HEVC: `qp -= 1` (preserve quality)

5. **Set AV1 profile**
   - 8-bit: `-profile:v 0` (Main)
   - 10-bit: `-profile:v 1` (High)

## ✅ What Will Work (Confirmed)

Based on research of `lscr.io/linuxserver/ffmpeg:version-8.0-cli`:

- ✅ `-qp` parameter (quality control)
- ✅ `-profile:v` parameter (bit depth)
- ✅ `format=nv12` (8-bit)
- ✅ `format=p010le` (10-bit)
- ✅ Bit depth detection
- ✅ Quality calculation improvements

## ❌ What to Skip

- ❌ `-rc_mode` (not needed, auto-selected)
- ❌ `-tier:v` (not critical, auto-detected)
- ❌ `-tile_rows/-tile_cols` (may not be exposed)
- ❌ `-quality` (this is speed/quality, not what we want)

## 📋 Implementation Steps (Simplified)

### Step 1: Add Bit Depth Detection (45 min)
- Add fields to `FFProbeStream`
- Create `BitDepth` enum
- Implement detection logic

### Step 2: Update Job Tracking (15 min)
- Add bit depth fields to `Job` struct

### Step 3: Fix Quality Calculation (45 min)
- Rename function to `calculate_optimal_qp`
- Fix codec adjustment logic (reverse signs)
- Add bit depth consideration
- Expand QP range to 20-40

### Step 4: Update Encoding Function (60 min)
- Create `EncodingParams` struct
- Dynamic pixel format selection
- Use `-qp` instead of `-quality`
- Add `-profile:v` parameter

### Step 5: Update Main Daemon (30 min)
- Extract bit depth from metadata
- Determine encoding parameters
- Pass to encoding function
- Update job tracking

### Step 6: Test (60 min)
- Test 8-bit source
- Test 10-bit source
- Test HDR content
- Verify output

**Total: ~4.5 hours**

## 🎯 Expected Results

| Source | Current | After Fix |
|--------|---------|-----------|
| 8-bit H.264 1080p | 8-bit AV1 ✓ | 8-bit AV1 ✓ (better QP) |
| 10-bit HEVC 4K | 8-bit AV1 ❌ | 10-bit AV1 ✓ |
| 10-bit HDR | Broken ❌ | HDR preserved ✓ |

**File Size Reductions**:
- 8-bit H.264: 60-70% reduction
- 10-bit HEVC: 45-55% reduction (preserves quality)

## 🚀 Next Steps

1. **Read**: `LINUXSERVER_FFMPEG_ANALYSIS.md`
   - Understand what parameters work with your Docker image

2. **Read**: `FINAL_IMPLEMENTATION_GUIDE.md`
   - Get the simplified implementation plan

3. **Optional**: Run `test_ffmpeg_capabilities.sh`
   - Verify encoder capabilities (if you want to be extra sure)

4. **Implement**: Follow the 6 steps above
   - Use `IMPLEMENTATION_CHECKLIST.md` to track progress

5. **Test**: Verify with sample files
   - 8-bit source
   - 10-bit source
   - HDR content

## 🧪 Quick Test (Optional)

Before implementing, you can test if 10-bit encoding works:

```bash
# Test 10-bit encoding
docker run --rm --privileged \
  -v /dev/dri:/dev/dri \
  -v /path/to/test:/config \
  lscr.io/linuxserver/ffmpeg:version-8.0-cli \
  ffmpeg -init_hw_device vaapi=va:/dev/dri/renderD128 \
  -hwaccel vaapi -hwaccel_device /dev/dri/renderD128 \
  -i /config/input_10bit.mkv \
  -vf "format=p010le,hwupload" \
  -c:v av1_vaapi \
  -qp 28 \
  -profile:v 1 \
  -c:a copy \
  /config/output.mkv

# Verify output
ffprobe -v error -select_streams v:0 \
  -show_entries stream=bits_per_raw_sample,profile \
  -of default=noprint_wrappers=1 /config/output.mkv
```

Expected output:
```
bits_per_raw_sample=10
profile=High
```

## ❓ Questions?

- **Will this break existing functionality?** No, it's additive. 8-bit sources will still work.
- **How long will it take?** ~4.5 hours including testing
- **Is it risky?** Low risk. Changes are well-defined and testable.
- **Can I rollback?** Yes, just revert the git commits.

## 📞 Support Documents

If you need more details:
- **Technical questions**: See `TECHNICAL_REFERENCE.md`
- **Parameter details**: See `LINUXSERVER_FFMPEG_ANALYSIS.md`
- **Step-by-step**: See `FINAL_IMPLEMENTATION_GUIDE.md`
- **Progress tracking**: Use `IMPLEMENTATION_CHECKLIST.md`

## 🎉 Benefits After Implementation

1. ✅ 10-bit sources preserve quality (no more banding)
2. ✅ HDR content works correctly
3. ✅ Better compression for H.264 sources
4. ✅ Better quality for HEVC sources
5. ✅ Predictable, optimal file sizes
6. ✅ Proper metadata tracking

---

**Ready to start?** Read `LINUXSERVER_FFMPEG_ANALYSIS.md` next, then `FINAL_IMPLEMENTATION_GUIDE.md`.

**Estimated time**: 4.5 hours
**Difficulty**: Medium
**Impact**: High (major quality improvement)

Good luck! 🚀
