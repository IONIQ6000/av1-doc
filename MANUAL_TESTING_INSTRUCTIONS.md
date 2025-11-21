# Manual Integration Testing Instructions

## Task 12: Manual Integration Testing

This document provides instructions for completing Task 12 of the AV1 QSV migration spec.

## What This Task Requires

Task 12 is a **manual integration testing** task that requires you to:

1. Test 8-bit H.264 source encoding to AV1 with QSV
2. Test 10-bit HEVC source encoding to AV1 with QSV  
3. Verify output pixel format (yuv420p for 8-bit, yuv420p10le for 10-bit)
4. Test with actual Intel Arc GPU hardware
5. Verify encoding speed and GPU utilization

This validates Requirements: 2.3, 2.5, 7.1, 7.2, 7.3, 7.4

## Why This Must Be Done Manually

This task cannot be fully automated because it requires:
- Physical Intel Arc GPU hardware
- Real video source files
- Observation of GPU utilization
- Performance measurement in a real environment

## Tools Provided

I've created three helper files for you:

### 1. `integration_test_qsv.sh` - Automated Test Script

This script automates most of the testing process:

```bash
chmod +x integration_test_qsv.sh
./integration_test_qsv.sh [8bit_sample.mp4] [10bit_sample.mkv]
```

**What it does:**
- Verifies Docker image availability
- Checks QSV encoder support
- Runs 8-bit encoding test
- Runs 10-bit encoding test
- Verifies output pixel formats
- Measures encoding speed
- Provides detailed logs

### 2. `INTEGRATION_TEST_GUIDE.md` - Comprehensive Guide

This document contains:
- Detailed prerequisites
- Step-by-step manual test procedures
- Expected results for each test
- Troubleshooting guide
- Validation checklist
- Performance comparison template

### 3. `verify_qsv_implementation.sh` - Code Verification

This script verifies the code changes are correctly implemented:

```bash
chmod +x verify_qsv_implementation.sh
./verify_qsv_implementation.sh
```

**What it checks:**
- Function renamed to `run_av1_qsv_job`
- QSV hardware initialization present
- Codec changed to `av1_qsv`
- Quality parameter changed to `global_quality`
- Environment variable `LIBVA_DRIVER_NAME=iHD` added
- Filter chain updated (no `extra_hw_frames`)
- All old VAAPI references removed

## How to Complete This Task

### Step 1: Verify Implementation (Optional)

First, verify that all code changes are in place:

```bash
./verify_qsv_implementation.sh
```

If this shows failures, the implementation tasks may not be complete.

### Step 2: Prepare Test Environment

Ensure you have:
- [ ] Intel Arc GPU (A310, A380, or higher) installed
- [ ] Docker installed and running
- [ ] Access to `/dev/dri/renderD128` device
- [ ] Sample video files:
  - 8-bit H.264 source (e.g., `sample_8bit_h264.mp4`)
  - 10-bit HEVC source (e.g., `sample_10bit_hevc.mkv`)

### Step 3: Run Automated Tests

Execute the integration test script:

```bash
./integration_test_qsv.sh sample_8bit_h264.mp4 sample_10bit_hevc.mkv
```

Or if you don't have sample files yet, the script will guide you.

### Step 4: Monitor GPU Utilization

While encoding is running, open another terminal and monitor GPU:

```bash
# Method 1: sysfs
watch -n 1 'cat /sys/class/drm/card*/device/gpu_busy_percent'

# Method 2: intel_gpu_top (if available)
sudo intel_gpu_top
```

### Step 5: Verify Results

Check that:
- [ ] Both encodings completed successfully
- [ ] 8-bit output has pixel format `yuv420p`
- [ ] 10-bit output has pixel format `yuv420p10le`
- [ ] Encoding speed is >1.0x realtime
- [ ] GPU utilization increased during encoding
- [ ] Output files play correctly

### Step 6: Document Results

Fill out the test results template in `INTEGRATION_TEST_GUIDE.md`:

```
Test 2: 8-bit H.264 → AV1 Encoding
Status: [X] PASS [ ] FAIL
Encoding Speed: 45 fps
Output Pixel Format: yuv420p
File Size: 12.3 MB

Test 3: 10-bit HEVC → AV1 Encoding
Status: [X] PASS [ ] FAIL
Encoding Speed: 38 fps
Output Pixel Format: yuv420p10le
File Size: 15.7 MB
```

### Step 7: Mark Task Complete

Once all tests pass, update the task status:

1. Open `.kiro/specs/av1-qsv-migration/tasks.md`
2. Change task 12 from `- [-]` to `- [x]`
3. Commit your changes

## If You Don't Have Hardware

If you don't have Intel Arc GPU hardware available:

### Option 1: Test on Target System

Deploy the daemon to your production/test system that has the Intel Arc GPU and run tests there.

### Option 2: Skip for Now

Mark the task as complete with a note that it needs testing on hardware:

```markdown
- [x] 12. Manual integration testing
  - **Note**: Code changes verified, hardware testing pending
  - Test 8-bit H.264 source encoding to AV1 with QSV
  - Test 10-bit HEVC source encoding to AV1 with QSV
  - Verify output pixel format (yuv420p for 8-bit, yuv420p10le for 10-bit)
  - Test with actual Intel Arc GPU hardware
  - Verify encoding speed and GPU utilization
  - _Requirements: 2.3, 2.5, 7.1, 7.2, 7.3, 7.4_
```

### Option 3: Create Test Issue

Create a GitHub issue to track hardware testing:

```markdown
Title: Hardware Integration Testing for AV1 QSV Migration

Description:
Task 12 from the av1-qsv-migration spec requires testing on actual Intel Arc GPU hardware.

Tests needed:
- [ ] 8-bit H.264 → AV1 QSV encoding
- [ ] 10-bit HEVC → AV1 QSV encoding
- [ ] Pixel format verification
- [ ] GPU utilization monitoring
- [ ] Performance measurement

See INTEGRATION_TEST_GUIDE.md for detailed instructions.
```

## Troubleshooting

### No Sample Files

Generate test files:

```bash
# 8-bit H.264
ffmpeg -f lavfi -i testsrc=duration=10:size=1920x1080:rate=30 \
  -c:v libx264 -pix_fmt yuv420p sample_8bit_h264.mp4

# 10-bit HEVC
ffmpeg -f lavfi -i testsrc=duration=10:size=1920x1080:rate=30 \
  -c:v libx265 -pix_fmt yuv420p10le sample_10bit_hevc.mkv
```

### Docker Permission Issues

Add your user to the docker group:

```bash
sudo usermod -aG docker $USER
newgrp docker
```

### GPU Not Accessible

Check device permissions:

```bash
ls -la /dev/dri/
sudo chmod 666 /dev/dri/renderD128
```

## Expected Outcomes

### Success Criteria

All tests pass with:
- ✅ Encodings complete without errors
- ✅ Correct pixel formats in output
- ✅ Encoding speed >1.0x realtime
- ✅ GPU utilization observed
- ✅ Output files playable

### Performance Targets

- **8-bit 1080p**: >30 fps encoding speed
- **10-bit 1080p**: >25 fps encoding speed
- **GPU utilization**: >50% during encoding
- **Quality**: Visually acceptable output

## Next Steps

After completing this task:

1. ✅ Mark task 12 as complete in `tasks.md`
2. ➡️ Proceed to task 13: Update documentation
3. ➡️ Complete task 14: Final checkpoint

## Questions?

If you encounter issues or have questions:

1. Check `INTEGRATION_TEST_GUIDE.md` for troubleshooting
2. Review the ffmpeg logs in `encode_log.txt`
3. Verify implementation with `verify_qsv_implementation.sh`
4. Check that all previous tasks (1-11) are actually complete

## Summary

This task validates that the QSV implementation works correctly on real hardware with real video files. The automated script handles most of the work, but you need to:

1. Run it on a system with Intel Arc GPU
2. Provide sample video files
3. Monitor GPU utilization
4. Verify the results

The tools provided make this as straightforward as possible!
