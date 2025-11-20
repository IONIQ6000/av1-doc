# Implementation Checklist

Use this checklist to track progress during implementation.

## Pre-Implementation

- [ ] Read and understand `AV1_QUALITY_IMPROVEMENT_SPEC.md`
- [ ] Review `IMPLEMENTATION_PLAN.md`
- [ ] Check `TECHNICAL_REFERENCE.md` for parameter details
- [ ] Verify GPU supports 10-bit AV1 encoding (`vainfo | grep AV1`)
- [ ] Verify FFmpeg version supports required parameters
- [ ] Create git branch for changes: `git checkout -b av1-quality-improvements`
- [ ] Backup current state: `git commit -am "Backup before AV1 improvements"`

## Step 1: Extend FFProbeStream Structure (15 min)

**File**: `crates/daemon/src/ffprobe.rs`

- [ ] Add `pix_fmt: Option<String>` field
- [ ] Add `bits_per_raw_sample: Option<String>` field
- [ ] Add `color_space: Option<String>` field
- [ ] Add `color_transfer: Option<String>` field
- [ ] Add `color_primaries: Option<String>` field
- [ ] Add `#[serde(rename = "...")]` attributes as needed
- [ ] Run `cargo check` - should compile ✓
- [ ] Commit: `git commit -am "Add bit depth fields to FFProbeStream"`

## Step 2: Create BitDepth Type and Detection Logic (30 min)

**File**: `crates/daemon/src/ffprobe.rs`

- [ ] Add `BitDepth` enum (Bit8, Bit10, Unknown)
- [ ] Derive Debug, Clone, Copy, PartialEq, Eq
- [ ] Implement `detect_bit_depth(&self) -> BitDepth` on FFProbeStream
  - [ ] Check `bits_per_raw_sample` field
  - [ ] Parse `pix_fmt` for "10" or "p010"
  - [ ] Check HDR metadata
  - [ ] Default to Bit8
- [ ] Implement `is_hdr_content(&self) -> bool` helper
  - [ ] Check `color_transfer` for "smpte2084" or "arib-std-b67"
  - [ ] Check `color_primaries` for "bt2020"
- [ ] Run `cargo check` - should compile ✓
- [ ] Test with sample file (manual test)
- [ ] Commit: `git commit -am "Add BitDepth detection logic"`

## Step 3: Extend Job Structure (15 min)

**File**: `crates/daemon/src/job.rs`

- [ ] Add `source_bit_depth: Option<u8>` field
- [ ] Add `source_pix_fmt: Option<String>` field
- [ ] Add `target_bit_depth: Option<u8>` field
- [ ] Add `av1_profile: Option<u8>` field
- [ ] Add `is_hdr: Option<bool>` field
- [ ] Update `Job::new()` to initialize new fields as None
- [ ] Run `cargo check` - should compile ✓
- [ ] Test JSON serialization (manual test)
- [ ] Commit: `git commit -am "Add bit depth tracking to Job struct"`

## Step 4: Create Encoding Parameters Structure (20 min)

**File**: `crates/daemon/src/ffmpeg_docker.rs`

- [ ] Add `EncodingParams` struct
  - [ ] `bit_depth: BitDepth`
  - [ ] `pixel_format: String`
  - [ ] `av1_profile: u8`
  - [ ] `qp: i32`
  - [ ] `is_hdr: bool`
- [ ] Derive Debug, Clone
- [ ] Create `determine_encoding_params()` function
  - [ ] Accept `meta: &FFProbeData, input_file: &Path`
  - [ ] Detect bit depth from video stream
  - [ ] Determine pixel format (nv12 or p010le)
  - [ ] Determine AV1 profile (0 or 1)
  - [ ] Call quality calculation (will update next)
  - [ ] Return EncodingParams
- [ ] Run `cargo check` - should compile ✓
- [ ] Commit: `git commit -am "Add EncodingParams structure"`

## Step 5: Update Quality Calculation Function (45 min)

**File**: `crates/daemon/src/ffmpeg_docker.rs`

- [ ] Rename `calculate_optimal_quality` to `calculate_optimal_qp`
- [ ] Update signature: add `bit_depth: BitDepth` parameter
- [ ] Update return type documentation (QP not "quality")
- [ ] Update base QP calculation
  - [ ] 4K: 26 (10-bit) or 28 (8-bit)
  - [ ] 1440p: 28 (10-bit) or 30 (8-bit)
  - [ ] 1080p: 30 (10-bit) or 32 (8-bit)
  - [ ] 720p: 32 (10-bit) or 34 (8-bit)
- [ ] **FIX codec adjustment logic** (CRITICAL)
  - [ ] H.264: `qp += 2` (more compression)
  - [ ] HEVC: `qp -= 1` (less compression)
  - [ ] VP9: `qp -= 1`
  - [ ] AV1: `qp += 0`
- [ ] Update bitrate efficiency thresholds
  - [ ] Very high (>0.6): `qp += 3`
  - [ ] High (0.4-0.6): `qp += 2`
  - [ ] Medium (0.2-0.4): `qp += 1`
  - [ ] Low (0.1-0.2): `qp += 0`
  - [ ] Very low (<0.1): `qp -= 1`
- [ ] Add bit depth adjustment
  - [ ] 10-bit: `qp -= 1`
- [ ] Update clamping: `qp.max(20).min(40)`
- [ ] Update all comments and documentation
- [ ] Update logging messages
- [ ] Run `cargo check` - should compile ✓
- [ ] Commit: `git commit -am "Fix and improve QP calculation"`

## Step 6: Update Encoding Function (60 min)

**File**: `crates/daemon/src/ffmpeg_docker.rs`

- [ ] Update `run_av1_vaapi_job` signature
  - [ ] Add `encoding_params: &EncodingParams` parameter
- [ ] Update filter chain
  - [ ] Replace hardcoded "nv12" with `encoding_params.pixel_format`
  - [ ] Use `format!("format={}", encoding_params.pixel_format)`
- [ ] **REMOVE old quality parameter**
  - [ ] Delete `-quality` lines
- [ ] **ADD new rate control parameters**
  - [ ] Add `-rc_mode` with value "CQP"
  - [ ] Add `-qp` with value from `encoding_params.qp`
- [ ] Add AV1 profile
  - [ ] Add `-profile:v` with value from `encoding_params.av1_profile`
- [ ] Add tier
  - [ ] Add `-tier:v` with value "0"
- [ ] Add tile configuration
  - [ ] Add `-tile_rows` with value "1"
  - [ ] Add `-tile_cols` with value "2"
- [ ] Update logging
  - [ ] Log bit depth information
  - [ ] Log QP value
  - [ ] Log profile
- [ ] Update `FFmpegResult` struct
  - [ ] Change `quality_used` to `qp_used` (or keep for compatibility)
- [ ] Run `cargo check` - should compile ✓
- [ ] Commit: `git commit -am "Update encoding function with proper parameters"`

## Step 7: Update Main Daemon Workflow (30 min)

**File**: `crates/cli-daemon/src/main.rs`

- [ ] After probing file, extract bit depth
  - [ ] Find video stream
  - [ ] Call `detect_bit_depth()`
  - [ ] Store in `job.source_bit_depth`
  - [ ] Store `is_hdr` in `job.is_hdr`
  - [ ] Store `pix_fmt` in `job.source_pix_fmt`
- [ ] Call `determine_encoding_params()`
  - [ ] Pass metadata and file path
  - [ ] Store result
- [ ] Update job with encoding params
  - [ ] Set `job.target_bit_depth`
  - [ ] Set `job.av1_profile`
  - [ ] Set `job.av1_quality` (QP value)
- [ ] Update logging
  - [ ] Log source bit depth
  - [ ] Log target bit depth
  - [ ] Log QP value
  - [ ] Log profile
  - [ ] Log HDR status
- [ ] Pass encoding params to `run_av1_vaapi_job()`
  - [ ] Add `&encoding_params` argument
- [ ] Run `cargo check` - should compile ✓
- [ ] Commit: `git commit -am "Update main daemon with bit depth handling"`

## Step 8: Update Expected Reduction Calculation (15 min)

**File**: `crates/daemon/src/ffmpeg_docker.rs`

- [ ] Update `calculate_expected_reduction` signature
  - [ ] Add `bit_depth: BitDepth` parameter
- [ ] Update QP-based reduction estimates
  - [ ] QP 20-24: 40-50%
  - [ ] QP 25-28: 50-60%
  - [ ] QP 29-32: 60-70%
  - [ ] QP 33-36: 70-75%
  - [ ] QP 37-40: 75-80%
- [ ] Add bit depth adjustment
  - [ ] 10-bit: Reduce expected compression by 5%
- [ ] Update function calls to pass bit depth
- [ ] Run `cargo check` - should compile ✓
- [ ] Commit: `git commit -am "Update expected reduction calculation"`

## Step 9: Full Compilation Check (10 min)

- [ ] Run `cargo clean`
- [ ] Run `cargo build --release`
- [ ] Verify no errors
- [ ] Verify no warnings (or document acceptable warnings)
- [ ] Commit: `git commit -am "Final compilation check passed"`

## Step 10: Testing & Validation (90 min)

### Test Case 1: 8-bit H.264 1080p
- [ ] Prepare test file (8-bit H.264 1080p)
- [ ] Run encoding
- [ ] Check output with ffprobe:
  ```bash
  ffprobe -v error -select_streams v:0 \
    -show_entries stream=pix_fmt,bits_per_raw_sample,profile \
    -of json output.mkv
  ```
- [ ] Verify: `bits_per_raw_sample` = "8"
- [ ] Verify: `profile` = "Main" or 0
- [ ] Verify: File size reduction 60-70%
- [ ] Visual inspection: No quality loss
- [ ] Check job JSON: All fields populated correctly

### Test Case 2: 10-bit HEVC 4K
- [ ] Prepare test file (10-bit HEVC 4K)
- [ ] Run encoding
- [ ] Check output with ffprobe
- [ ] Verify: `bits_per_raw_sample` = "10"
- [ ] Verify: `profile` = "High" or 1
- [ ] Verify: File size reduction 45-55%
- [ ] Visual inspection: No banding, quality preserved
- [ ] Check job JSON: Bit depth fields correct

### Test Case 3: 10-bit HDR Content
- [ ] Prepare test file (10-bit HDR)
- [ ] Run encoding
- [ ] Check output with ffprobe
- [ ] Verify: `bits_per_raw_sample` = "10"
- [ ] Verify: HDR metadata preserved:
  ```bash
  ffprobe -v error -select_streams v:0 \
    -show_entries stream=color_space,color_transfer,color_primaries \
    -of json output.mkv
  ```
- [ ] Verify: `color_transfer` = "smpte2084" or "arib-std-b67"
- [ ] Verify: `color_primaries` = "bt2020"
- [ ] Visual inspection: HDR looks correct
- [ ] Check job JSON: `is_hdr` = true

### Test Case 4: Low Bitrate Source
- [ ] Prepare test file (low bitrate, <0.1 bpppf)
- [ ] Run encoding
- [ ] Verify: QP value is lower (less compression)
- [ ] Verify: No over-compression artifacts
- [ ] Check logs: QP adjustment logged correctly

### Test Case 5: Various Codecs
- [ ] Test H.264 source → Verify QP adjustment (+2)
- [ ] Test HEVC source → Verify QP adjustment (-1)
- [ ] Test VP9 source → Verify QP adjustment (-1)
- [ ] Check logs: Codec adjustments logged correctly

### General Validation
- [ ] All test files encode successfully
- [ ] No encoding failures
- [ ] Logs are clear and informative
- [ ] Job JSON files contain all new fields
- [ ] QP values in range 20-40
- [ ] File sizes meet expectations
- [ ] No visual quality degradation

## Step 11: Documentation Updates (20 min)

- [ ] Update `README.md`
  - [ ] Add section on bit depth preservation
  - [ ] Document QP parameter usage
  - [ ] Add examples of expected results
- [ ] Update code comments
  - [ ] Ensure all new functions documented
  - [ ] Update existing comments if needed
- [ ] Create CHANGELOG entry
  - [ ] List all improvements
  - [ ] Note breaking changes (if any)
- [ ] Commit: `git commit -am "Update documentation"`

## Post-Implementation

- [ ] Final review of all changes
- [ ] Run full test suite again
- [ ] Create pull request or merge to main
- [ ] Tag release: `git tag -a v1.0.0 -m "AV1 quality improvements"`
- [ ] Monitor first production runs
- [ ] Document any issues encountered
- [ ] Update this checklist with lessons learned

## Success Metrics

After implementation, verify these metrics:

- [ ] ✅ 100% of 8-bit sources produce 8-bit output
- [ ] ✅ 100% of 10-bit sources produce 10-bit output
- [ ] ✅ HDR content properly flagged and preserved
- [ ] ✅ QP values in expected range (20-40)
- [ ] ✅ Average file size reduction: 50-65%
- [ ] ✅ No visual quality degradation
- [ ] ✅ No encoding failures
- [ ] ✅ All metadata tracked in job files
- [ ] ✅ Logs are clear and informative

## Rollback Plan (If Needed)

If critical issues arise:

- [ ] Stop all encoding jobs
- [ ] Document the issue
- [ ] Revert to previous commit: `git revert HEAD`
- [ ] Or checkout previous branch: `git checkout main`
- [ ] Rebuild: `cargo build --release`
- [ ] Restart daemon with old version
- [ ] Analyze what went wrong
- [ ] Update plan and retry

## Notes Section

Use this space to track issues, observations, or improvements discovered during implementation:

```
Date: ___________
Issue: 
Solution:

Date: ___________
Observation:
Action:

Date: ___________
Improvement idea:
```

---

**Estimated Total Time**: 5.5 hours
**Actual Time**: _________ (fill in after completion)

**Implementation Status**: 
- [ ] Not Started
- [ ] In Progress
- [ ] Testing
- [ ] Complete
- [ ] Deployed

**Final Sign-off**: _________ (date completed)
