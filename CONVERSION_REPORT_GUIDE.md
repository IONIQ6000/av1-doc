# Conversion Report Guide

## Overview

After every successful AV1 conversion, the system generates a comprehensive `.av1-conversion-report.txt` file next to the converted video. This report contains detailed information about the entire conversion process to help you verify the conversion worked correctly.

## Report Location

For a video file named `Movie.mkv`, the report will be:
```
Movie.av1-conversion-report.txt
```

The report is placed in the same directory as the converted video file.

## Report Sections

### 1. Job Information

Basic information about the conversion job:
- **Job ID**: Unique identifier for tracking
- **Status**: Success/Failed/Skipped
- **Source/Output Files**: Full paths
- **Timestamps**: Start, completion, and duration
- **Duration**: Total time taken (hours, minutes, seconds)

**What to check:**
- ✓ Status should be "Success"
- ✓ Duration seems reasonable for file size

### 2. Source Analysis

Detailed analysis of the original video file:

**Video Stream:**
- Codec (h264, hevc, vp9, etc.)
- Resolution (1920x1080, 3840x2160, etc.)
- Pixel format (yuv420p, yuv420p10le, etc.)
- Bit depth (8-bit or 10-bit)
- HDR status and color information
- Frame rate
- Bitrate

**Audio/Subtitle Streams:**
- Number of tracks
- Codec for each track
- Language codes

**Container:**
- Format (matroska, mp4, etc.)
- Muxing application used
- Writing library

**What to check:**
- ✓ Source codec is correctly identified
- ✓ Bit depth matches your expectations (8-bit or 10-bit)
- ✓ HDR is detected if source is HDR
- ✓ All audio/subtitle tracks are listed

### 3. Source Classification

How the system classified your source file:

**Classification Types:**
- **WebLike**: Web downloads (WEB-DL, WEBRIP, streaming sources)
- **DiscLike**: Blu-ray/DVD rips (REMUX, BDRIP)
- **Unknown**: Couldn't determine (uses conservative strategy)

**Confidence Score:**
- Positive score (>0.4): WebLike
- Negative score (<-0.3): DiscLike
- Between: Unknown

**Detection Signals:**
Lists all the clues used to classify the source:
- Filename tokens (WEB-DL, BLURAY, etc.)
- Audio codecs (AAC=web, TrueHD=disc)
- Stream counts (single audio=web, multiple=disc)
- Variable frame rate (VFR=web)
- And more...

**Encoding Strategy:**
- **WEB**: Special VFR handling, timestamp fixes
- **DISC**: Standard CFR processing
- **UNKNOWN**: Conservative approach

**What to check:**
- ✓ Classification matches your source type
- ✓ If web source, VFR handling is enabled
- ✓ Detection signals make sense

**⚠️ IMPORTANT:** Incorrect classification can cause corruption!
- Web sources NEED VFR handling
- If classified wrong, report it for improvement

### 4. Encoding Parameters

Details about how the video was encoded:

**Hardware:**
- Encoder: Intel QSV (Quick Sync Video)
- Codec: av1_qsv
- Device: /dev/dri/renderD128
- Driver: iHD

**Quality Settings:**
- **Target Bit Depth**: 8-bit or 10-bit
- **Pixel Format**: nv12 (8-bit) or p010le (10-bit)
- **AV1 Profile**: 0 (main) for both 8-bit and 10-bit
- **Quality (QP)**: 20-40 (lower = higher quality)
- **HDR Encoding**: Yes/No

**Filter Chain:**
Shows the video processing pipeline:
1. Pad to even dimensions
2. Set aspect ratio
3. Convert pixel format
4. Upload to GPU

**Stream Handling:**
- Video: Transcoded to AV1
- Audio: Copied (no re-encoding)
- Subtitles: Copied
- Chapters: Preserved
- Metadata: Preserved
- Russian tracks: Removed

**What to check:**
- ✓ Bit depth matches source (8-bit→8-bit, 10-bit→10-bit)
- ✓ HDR encoding enabled for HDR sources
- ✓ QP value is reasonable (26-32 typical)
- ✓ Audio/subtitles are copied (not re-encoded)

### 5. File Size Comparison

Space savings from the conversion:

- **Original Size**: Size before conversion (GB)
- **New Size**: Size after conversion (GB)
- **Space Saved**: Difference (GB and %)
- **Compression**: How many times smaller

**What to check:**
- ✓ Space saved is significant (typically 40-70%)
- ✓ New file isn't larger than original
- ✓ Compression ratio seems reasonable

**Typical Results:**
- H.264 sources: 60-70% reduction
- HEVC sources: 40-50% reduction
- High bitrate sources: More compression
- Low bitrate sources: Less compression

### 6. Output Validation

Comprehensive checks to ensure the output isn't corrupted:

**10 Validation Checks:**
1. ✓ File exists and not empty
2. ✓ FFprobe can read file (not corrupted)
3. ✓ Video stream exists and valid
4. ✓ Codec is AV1 as expected
5. ✓ Bit depth matches target
6. ✓ Pixel format is correct
7. ✓ Dimensions are valid
8. ✓ Frame rate consistent (no VFR corruption)
9. ✓ Audio streams preserved
10. ✓ Bitrate within expected range

**Status:**
- **✓ PASSED**: All checks passed, file is good
- **✗ FAILED**: Critical issues found, file may be corrupted

**Issues vs Warnings:**
- **Issues**: Critical problems (job fails)
- **Warnings**: Non-critical concerns (job succeeds)

**What to check:**
- ✓ Validation status is PASSED
- ✓ No critical issues listed
- ✓ Warnings (if any) are acceptable

**⚠️ Common Warnings:**
- Bit depth differs: Usually safe, check playback
- VFR detected: May indicate timestamp issues
- Odd dimensions: May cause playback issues

### 7. FFmpeg Encoding Log

Last 50 lines of FFmpeg output showing:
- Frame progress
- Encoding speed (fps)
- Quality (q=)
- Bitrate
- Encoding statistics

**What to check:**
- ✓ Encoding completed (shows final frame)
- ✓ Speed was reasonable (>1.0x)
- ✓ No error messages
- ✓ Quality value matches QP setting

## How to Use This Report

### 1. Verify Conversion Success

Check these key indicators:
```
✓ Status: Success
✓ Validation Status: PASSED
✓ Space Saved: 40-70%
✓ Classification: Correct for your source
```

### 2. Troubleshoot Issues

If something seems wrong:

**Video won't play:**
- Check validation section for issues
- Look for VFR corruption warnings
- Verify bit depth matches

**Quality looks bad:**
- Check QP value (should be 26-32)
- Verify source bitrate wasn't too low
- Check if HDR was preserved

**File size too large:**
- Check source codec (HEVC compresses less)
- Verify QP wasn't too low
- Check if source was already efficient

**Audio/subtitles missing:**
- Check stream handling section
- Verify tracks were in source
- Check if Russian tracks were removed

### 3. Report Problems

If you find issues, include:
- The full conversion report
- Description of the problem
- Expected vs actual behavior
- Source file characteristics

## Example: Good Conversion

```
Status:           Success
Classification:   WebLike (score: 0.65)
Encoding Strategy: WEB (VFR handling enabled)
Quality (QP):     28
Space Saved:      23.63 GB (55.8% reduction)
Validation:       ✓ PASSED (no warnings)
```

## Example: Problem Conversion

```
Status:           Success
Classification:   Unknown (score: 0.15)
Encoding Strategy: CONSERVATIVE
Validation:       ✓ PASSED
Warnings:
  ⚠ Variable frame rate detected in output
  ⚠ Output bit depth differs from expected
```

**Action:** Check playback carefully, may have VFR issues

## Tips

1. **Keep reports**: They're useful for troubleshooting
2. **Check classification**: Incorrect classification can cause corruption
3. **Monitor warnings**: They indicate potential issues
4. **Compare results**: Similar sources should have similar compression
5. **Report misclassifications**: Helps improve the system

## File Naming

Reports use this naming pattern:
```
Original:  Movie.2023.1080p.WEB-DL.mkv
Report:    Movie.2023.1080p.WEB-DL.av1-conversion-report.txt
```

The report stays with the video file even if you move it.

## Privacy Note

Reports contain:
- File paths (may include personal directory names)
- Encoding parameters
- FFmpeg output

Keep reports private if your file paths contain sensitive information.
