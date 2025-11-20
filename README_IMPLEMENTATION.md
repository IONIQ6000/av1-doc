# AV1 Quality Improvements - Complete Implementation Package

## 📋 Document Overview

This package contains everything needed to implement proper bit depth preservation and optimized quality parameters for AV1 encoding.

### Quick Start

1. **Read First**: `IMPLEMENTATION_SUMMARY.md` - High-level overview
2. **Understand**: `AV1_QUALITY_IMPROVEMENT_SPEC.md` - Detailed specification
3. **Follow**: `IMPLEMENTATION_PLAN.md` - Step-by-step guide
4. **Track**: `IMPLEMENTATION_CHECKLIST.md` - Progress tracking
5. **Reference**: `TECHNICAL_REFERENCE.md` - Parameter details
6. **Quick Lookup**: `QUICK_REFERENCE.md` - Key changes at a glance
7. **Visualize**: `ENCODING_FLOW_DIAGRAM.md` - Flow diagrams

## 🎯 What's Being Fixed

### Critical Issues
1. ❌ **All output is 8-bit** → ✅ Preserve source bit depth (8-bit or 10-bit)
2. ❌ **Wrong quality parameter** → ✅ Use correct QP with CQP rate control
3. ❌ **No AV1 profile set** → ✅ Set profile based on bit depth
4. ❌ **Quality logic backwards** → ✅ Fix codec adjustment logic

### Expected Improvements
- 10-bit sources preserve quality (no more banding)
- HDR content works correctly
- Better compression for H.264 sources
- Better quality preservation for HEVC sources
- Predictable, optimal file sizes

## 📊 Expected Results

| Source Type | Before | After |
|-------------|--------|-------|
| 8-bit H.264 | 8-bit output ✓ | 8-bit output ✓ (better compression) |
| 10-bit HEVC | 8-bit output ❌ (quality loss) | 10-bit output ✓ (quality preserved) |
| 10-bit HDR | 8-bit output ❌ (broken) | 10-bit output ✓ (HDR preserved) |

## 🔧 Files to Modify

1. **`crates/daemon/src/ffprobe.rs`** (~45 min)
   - Add bit depth detection fields
   - Implement detection logic

2. **`crates/daemon/src/job.rs`** (~15 min)
   - Add bit depth tracking fields

3. **`crates/daemon/src/ffmpeg_docker.rs`** (~90 min)
   - Create EncodingParams structure
   - Fix quality calculation (CRITICAL)
   - Update encoding function

4. **`crates/cli-daemon/src/main.rs`** (~30 min)
   - Extract and use bit depth
   - Update workflow

**Total estimated time**: 5.5 hours (including testing)

## 🚀 Implementation Steps

### Phase 1: Data Structures (1 hour)
1. Add bit depth fields to FFProbeStream
2. Create BitDepth enum and detection logic
3. Extend Job structure
4. Create EncodingParams structure

### Phase 2: Quality Calculation (45 min)
1. Fix codec adjustment logic (backwards currently)
2. Add bit depth consideration
3. Expand QP range (20-40)
4. Refine bitrate efficiency thresholds

### Phase 3: Encoding Pipeline (1 hour)
1. Dynamic pixel format selection (nv12 vs p010le)
2. Replace `-quality` with `-qp` and `-rc_mode CQP`
3. Add AV1 profile, tier, tiles
4. Update logging

### Phase 4: Integration (30 min)
1. Update main daemon workflow
2. Extract and pass bit depth
3. Update job tracking

### Phase 5: Testing (1.5 hours)
1. Test 8-bit sources
2. Test 10-bit sources
3. Test HDR content
4. Validate quality and file sizes

### Phase 6: Documentation (30 min)
1. Update README
2. Update code comments
3. Create changelog

## 📖 Document Guide

### For Understanding
- **`IMPLEMENTATION_SUMMARY.md`**: Start here for overview
- **`AV1_QUALITY_IMPROVEMENT_SPEC.md`**: Full technical specification
- **`ENCODING_FLOW_DIAGRAM.md`**: Visual representation

### For Implementation
- **`IMPLEMENTATION_PLAN.md`**: Detailed step-by-step guide
- **`IMPLEMENTATION_CHECKLIST.md`**: Track your progress
- **`QUICK_REFERENCE.md`**: Quick lookup for key changes

### For Reference
- **`TECHNICAL_REFERENCE.md`**: Parameter details, QP ranges, pixel formats

## 🔑 Key Technical Changes

### Bit Depth Detection
```rust
// Detect from metadata
pub fn detect_bit_depth(&self) -> BitDepth {
    if bits_per_raw_sample == "10" { BitDepth::Bit10 }
    else if pix_fmt.contains("10") { BitDepth::Bit10 }
    else { BitDepth::Bit8 }
}
```

### Dynamic Encoding
```rust
// 8-bit: nv12 + profile 0
// 10-bit: p010le + profile 1
let (format, profile) = match bit_depth {
    BitDepth::Bit8 => ("nv12", 0),
    BitDepth::Bit10 => ("p010le", 1),
};
```

### Correct Quality Parameters
```rust
// OLD: Wrong
ffmpeg_args.push("-quality".to_string());
ffmpeg_args.push("25".to_string());

// NEW: Correct
ffmpeg_args.push("-rc_mode".to_string());
ffmpeg_args.push("CQP".to_string());
ffmpeg_args.push("-qp".to_string());
ffmpeg_args.push("28".to_string());
ffmpeg_args.push("-profile:v".to_string());
ffmpeg_args.push("1".to_string()); // 10-bit
```

### Fixed Quality Logic
```rust
// OLD: Backwards
match codec {
    "hevc" => quality += 1,  // WRONG!
}

// NEW: Correct
match codec {
    "h264" => qp += 2,  // More compression
    "hevc" => qp -= 1,  // Preserve quality
}
```

## ✅ Success Criteria

Implementation is successful when:

1. ✅ 8-bit sources → 8-bit AV1 output
2. ✅ 10-bit sources → 10-bit AV1 output
3. ✅ HDR metadata preserved
4. ✅ QP values in range 20-40
5. ✅ File sizes: 35-75% reduction (depending on source)
6. ✅ No visual quality degradation
7. ✅ All metadata tracked in job files

## 🧪 Testing Commands

### Verify bit depth
```bash
ffprobe -v error -select_streams v:0 \
  -show_entries stream=bits_per_raw_sample,pix_fmt,profile \
  -of json output.mkv
```

### Verify HDR
```bash
ffprobe -v error -select_streams v:0 \
  -show_entries stream=color_transfer,color_primaries \
  -of json output.mkv
```

### Expected output for 10-bit
```json
{
  "streams": [{
    "bits_per_raw_sample": "10",
    "pix_fmt": "yuv420p10le",
    "profile": "High"
  }]
}
```

## 🎓 Learning Resources

### Understanding AV1
- AV1 profiles: Main (8-bit), High (10-bit), Professional (12-bit)
- QP range: 0-255 (lower = better quality, larger file)
- Practical QP: 20-40 for most content

### Understanding VAAPI
- Hardware acceleration for Intel GPUs
- Pixel formats: nv12 (8-bit), p010le (10-bit)
- Rate control modes: CQP (quality), CBR (bitrate), VBR (variable)

### Understanding Bit Depth
- 8-bit: 256 levels per color channel (standard)
- 10-bit: 1024 levels per color channel (better gradients)
- HDR requires 10-bit minimum

## 🐛 Troubleshooting

### Issue: Output is still 8-bit
- Check: Is p010le being used in filter chain?
- Check: Is profile:v set to 1?
- Check: Does GPU support 10-bit? (`vainfo | grep AV1`)

### Issue: Encoding fails
- Check: FFmpeg version supports parameters
- Check: GPU device accessible
- Check: Docker image is correct version

### Issue: File sizes too large
- Check: QP value (should be 20-40)
- Check: Rate control mode is CQP
- Check: Quality calculation logic

### Issue: Quality loss
- Check: Bit depth preserved?
- Check: QP not too high?
- Check: Codec adjustments correct?

## 📞 Support

If you encounter issues:

1. Check the troubleshooting section
2. Review logs for error messages
3. Verify GPU capabilities with `vainfo`
4. Test with known-good sample files
5. Refer to TECHNICAL_REFERENCE.md for parameter details

## 🔄 Rollback Plan

If critical issues arise:

```bash
# Revert changes
git revert HEAD

# Or checkout previous version
git checkout main

# Rebuild
cargo build --release

# Restart daemon
systemctl restart av1d
```

## 📝 Notes

- Take breaks between major steps
- Commit after each successful step
- Test incrementally
- Keep detailed notes of issues
- Update checklist as you go

## 🎉 After Implementation

Once complete:

1. Monitor first production runs
2. Verify file sizes and quality
3. Check logs for any issues
4. Update documentation with findings
5. Share results and lessons learned

## 📚 Document Versions

- **v1.0** - Initial specification (current)
- Created: [Date]
- Last updated: [Date]

---

**Ready to start?** 

1. Read `IMPLEMENTATION_SUMMARY.md`
2. Follow `IMPLEMENTATION_PLAN.md`
3. Track progress in `IMPLEMENTATION_CHECKLIST.md`
4. Reference `TECHNICAL_REFERENCE.md` as needed

**Estimated time**: 5.5 hours
**Difficulty**: Medium
**Impact**: High (major quality improvement)

Good luck! 🚀
