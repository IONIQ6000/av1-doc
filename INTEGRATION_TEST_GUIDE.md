# AV1 QSV Integration Test Guide

## Overview

This guide provides instructions for manually testing the AV1 QSV migration implementation. These tests verify that the system correctly encodes both 8-bit and 10-bit content using Intel Quick Sync Video on Intel Arc GPUs.

## Prerequisites

### Hardware Requirements
- Intel Arc GPU (A310, A380, or higher)
- System with `/dev/dri/renderD128` device available

### Software Requirements
- Docker installed and running
- Access to the `lscr.io/linuxserver/ffmpeg:version-8.0-cli` image
- Sample video files:
  - 8-bit H.264 source file
  - 10-bit HEVC source file

### Verify Hardware

Check that your Intel Arc GPU is available:

```bash
ls -la /dev/dri/
```

You should see `renderD128` (or similar render device).

Check GPU information:

```bash
lspci | grep -i vga
```

## Test Execution

### Automated Test Script

The `integration_test_qsv.sh` script automates most of the testing process:

```bash
# Make script executable
chmod +x integration_test_qsv.sh

# Run with default sample file names
./integration_test_qsv.sh

# Or specify custom sample files
./integration_test_qsv.sh path/to/8bit_sample.mp4 path/to/10bit_sample.mkv
```

### Manual Test Procedure

If you prefer to run tests manually or need to troubleshoot:

#### Test 1: Verify Docker Image and QSV Support

```bash
# Pull the Docker image
docker pull lscr.io/linuxserver/ffmpeg:version-8.0-cli

# Verify av1_qsv encoder is available
docker run --rm --privileged \
  --device /dev/dri:/dev/dri \
  -e LIBVA_DRIVER_NAME=iHD \
  lscr.io/linuxserver/ffmpeg:version-8.0-cli \
  -hide_banner -encoders | grep av1_qsv
```

Expected output should include:
```
 V..... av1_qsv              AV1 (Intel Quick Sync Video acceleration)
```

#### Test 2: 8-bit H.264 → 8-bit AV1 Encoding

```bash
# Analyze source file
ffprobe -v error -select_streams v:0 \
  -show_entries stream=codec_name,pix_fmt,width,height,bit_depth \
  -of default=noprint_wrappers=1 sample_8bit_h264.mp4

# Encode with QSV
docker run --rm --privileged \
  --user root \
  --device /dev/dri:/dev/dri \
  -e LIBVA_DRIVER_NAME=iHD \
  -v "$(pwd):/data" \
  lscr.io/linuxserver/ffmpeg:version-8.0-cli \
  -init_hw_device qsv=hw:/dev/dri/renderD128 \
  -filter_hw_device hw \
  -i /data/sample_8bit_h264.mp4 \
  -vf "format=nv12,hwupload" \
  -c:v av1_qsv \
  -global_quality 30 \
  -profile:v main \
  -c:a copy \
  -y /data/output_8bit_qsv.mkv

# Verify output pixel format
ffprobe -v error -select_streams v:0 \
  -show_entries stream=pix_fmt \
  -of default=noprint_wrappers=1:nokey=1 output_8bit_qsv.mkv
```

**Expected Results:**
- Encoding completes successfully
- Output pixel format: `yuv420p`
- Encoding speed: >1.0x realtime (check fps in output)
- No errors in ffmpeg output

#### Test 3: 10-bit HEVC → 10-bit AV1 Encoding

```bash
# Analyze source file
ffprobe -v error -select_streams v:0 \
  -show_entries stream=codec_name,pix_fmt,width,height,bit_depth \
  -of default=noprint_wrappers=1 sample_10bit_hevc.mkv

# Encode with QSV
docker run --rm --privileged \
  --user root \
  --device /dev/dri:/dev/dri \
  -e LIBVA_DRIVER_NAME=iHD \
  -v "$(pwd):/data" \
  lscr.io/linuxserver/ffmpeg:version-8.0-cli \
  -init_hw_device qsv=hw:/dev/dri/renderD128 \
  -filter_hw_device hw \
  -i /data/sample_10bit_hevc.mkv \
  -vf "format=p010le,hwupload" \
  -c:v av1_qsv \
  -global_quality 30 \
  -profile:v main \
  -c:a copy \
  -y /data/output_10bit_qsv.mkv

# Verify output pixel format
ffprobe -v error -select_streams v:0 \
  -show_entries stream=pix_fmt \
  -of default=noprint_wrappers=1:nokey=1 output_10bit_qsv.mkv
```

**Expected Results:**
- Encoding completes successfully
- Output pixel format: `yuv420p10le`
- Encoding speed: >1.0x realtime
- No errors in ffmpeg output

#### Test 4: GPU Utilization Monitoring

While encoding is running, monitor GPU utilization in a separate terminal:

```bash
# Method 1: Using sysfs
watch -n 1 'cat /sys/class/drm/card*/device/gpu_busy_percent'

# Method 2: Using intel_gpu_top (if available)
sudo intel_gpu_top

# Method 3: Using radeontop (alternative)
radeontop
```

**Expected Results:**
- GPU utilization should increase during encoding
- Video engine should show activity
- Utilization should be >50% during active encoding

## Validation Checklist

After running the tests, verify the following:

### Functional Requirements

- [ ] **Requirement 2.3**: 10-bit encoding completes successfully
  - Output file exists and is playable
  - No errors during encoding

- [ ] **Requirement 2.5**: 8-bit output has correct pixel format
  - `ffprobe` shows `yuv420p` for 8-bit output
  - `ffprobe` shows `yuv420p10le` for 10-bit output

- [ ] **Requirement 7.1**: Encoding speed is acceptable
  - 8-bit encoding speed: _____ fps (should be >30fps for 1080p)
  - Compare with previous VAAPI implementation if available

- [ ] **Requirement 7.2**: 10-bit encoding uses hardware acceleration
  - GPU utilization observed during encoding
  - Encoding speed indicates hardware acceleration (not software)

- [ ] **Requirement 7.3**: Progress metrics are reported
  - ffmpeg output shows frame count
  - ffmpeg output shows fps
  - ffmpeg output shows speed multiplier

- [ ] **Requirement 7.4**: Total time and speed are logged
  - Encoding completion message includes duration
  - Average speed is calculated and displayed

### Quality Verification

- [ ] Output files play correctly in media player
- [ ] Visual quality is acceptable (no obvious artifacts)
- [ ] Audio is preserved correctly
- [ ] File size is reasonable (AV1 should be smaller than source)

### Performance Comparison

If you have previous VAAPI implementation results, compare:

| Metric | VAAPI (8-bit) | QSV (8-bit) | QSV (10-bit) |
|--------|---------------|-------------|--------------|
| Encoding Speed (fps) | _____ | _____ | _____ |
| GPU Utilization (%) | _____ | _____ | _____ |
| Output File Size (MB) | _____ | _____ | _____ |
| Encoding Time (s) | _____ | _____ | _____ |

## Troubleshooting

### Error: "Cannot load libmfx"

**Cause**: Intel Media SDK / VPL not available in container

**Solution**: Ensure you're using the correct Docker image:
```bash
docker pull lscr.io/linuxserver/ffmpeg:version-8.0-cli
```

### Error: "No such device"

**Cause**: GPU device not accessible

**Solution**: 
- Check `/dev/dri` exists: `ls -la /dev/dri/`
- Ensure Docker has device access: `--device /dev/dri:/dev/dri`
- Check permissions on render device

### Error: "Encoder not found"

**Cause**: av1_qsv encoder not available

**Solution**:
- Verify encoder list: `docker run ... -encoders | grep av1`
- Check LIBVA_DRIVER_NAME is set: `-e LIBVA_DRIVER_NAME=iHD`

### Low Encoding Speed

**Possible Causes**:
- GPU not being used (falling back to software)
- Insufficient GPU resources
- Thermal throttling

**Solutions**:
- Monitor GPU utilization during encoding
- Check GPU temperature and clock speeds
- Verify hardware initialization succeeded in ffmpeg output

### Pixel Format Mismatch

**Cause**: Wrong pixel format in filter chain

**Solution**:
- 8-bit sources: Use `format=nv12`
- 10-bit sources: Use `format=p010le`
- Verify source bit depth with `ffprobe`

## Sample File Acquisition

If you need sample files for testing:

### 8-bit H.264 Sample

```bash
# Generate a test pattern (requires ffmpeg locally)
ffmpeg -f lavfi -i testsrc=duration=10:size=1920x1080:rate=30 \
  -c:v libx264 -pix_fmt yuv420p \
  sample_8bit_h264.mp4
```

### 10-bit HEVC Sample

```bash
# Generate a 10-bit test pattern
ffmpeg -f lavfi -i testsrc=duration=10:size=1920x1080:rate=30 \
  -c:v libx265 -pix_fmt yuv420p10le \
  sample_10bit_hevc.mkv
```

Or download sample files from:
- [Kodi Sample Files](https://kodi.wiki/view/Samples)
- [JellyFin Test Media](https://github.com/jellyfin/jellyfin-test-media)

## Test Results Template

Document your test results:

```
=== AV1 QSV Integration Test Results ===

Date: _______________
Tester: _______________
Hardware: _______________
OS: _______________

Test 1: Docker Image Verification
Status: [ ] PASS [ ] FAIL
Notes: _______________

Test 2: 8-bit H.264 → AV1 Encoding
Status: [ ] PASS [ ] FAIL
Encoding Speed: _____ fps
Output Pixel Format: _____
File Size: _____ MB
Notes: _______________

Test 3: 10-bit HEVC → AV1 Encoding
Status: [ ] PASS [ ] FAIL
Encoding Speed: _____ fps
Output Pixel Format: _____
File Size: _____ MB
Notes: _______________

Test 4: GPU Utilization
Status: [ ] PASS [ ] FAIL
Peak GPU Usage: _____ %
Notes: _______________

Overall Result: [ ] ALL TESTS PASSED [ ] SOME FAILURES

Additional Notes:
_______________
```

## Next Steps

After completing integration testing:

1. Document any issues found in GitHub issues
2. Update the tasks.md file to mark task 12 as complete
3. Proceed to task 13 (documentation updates) if all tests pass
4. If tests fail, investigate and fix issues before proceeding
