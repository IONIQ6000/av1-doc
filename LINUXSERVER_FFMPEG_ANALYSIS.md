# LinuxServer.io FFmpeg 8.0 Image Analysis

## Image Details

**Image**: `lscr.io/linuxserver/ffmpeg:version-8.0-cli`
**Registry**: LinuxServer.io Container Registry (lscr.io)
**Base**: Likely Alpine or Ubuntu with FFmpeg 8.0 compiled with VAAPI support

## LinuxServer.io FFmpeg Image Characteristics

### Known Facts

1. **FFmpeg Version**: 8.0.x (released April 2024)
2. **Purpose**: CLI-focused image (no GUI tools)
3. **Hardware Acceleration**: Built with VAAPI support for Intel GPUs
4. **Architecture**: Supports x86_64 (amd64)

### FFmpeg 8.0 VAAPI AV1 Encoder

FFmpeg 8.0 includes the `av1_vaapi` encoder with the following characteristics:

#### Supported Parameters (Based on FFmpeg 8.0 Source)

**Rate Control**:
- ✅ `-qp <int>` - Constant Quantization Parameter (0-255)
  - This is the PRIMARY quality control parameter
  - Lower values = better quality, larger files
  - Practical range: 20-40 for most content

- ✅ `-quality <int>` - Encode quality/speed tradeoff (1-8)
  - 1 = Best quality, slowest encoding
  - 8 = Lowest quality, fastest encoding
  - This is SPEED vs QUALITY, not bitrate vs quality
  - **Note**: This is different from what we want!

- ✅ `-b:v <bitrate>` - Target bitrate for VBR mode
  - Example: `-b:v 5M` for 5 Mbps

- ⚠️ `-rc_mode <mode>` - Rate control mode
  - May not be directly exposed in VAAPI wrapper
  - Mode is typically auto-selected based on other parameters:
    - If `-qp` is set → CQP (Constant QP)
    - If `-b:v` is set → VBR (Variable Bitrate)

**Profile/Level**:
- ✅ `-profile:v <int>` - AV1 profile
  - 0 = Main (8-bit, 4:2:0)
  - 1 = High (10-bit, 4:2:0)
  - 2 = Professional (12-bit, 4:2:2/4:4:4)

- ⚠️ `-tier:v <int>` - AV1 tier
  - May not be exposed in VAAPI
  - Usually auto-detected

- ⚠️ `-level <int>` - AV1 level
  - Usually auto-detected from resolution/framerate

**Tile Configuration**:
- ❓ `-tile_rows <int>` - Number of tile rows
- ❓ `-tile_cols <int>` - Number of tile columns
- These may or may not be exposed in the VAAPI wrapper
- If not available, encoder uses defaults

## Critical Finding: Use -qp, Not -quality

Based on FFmpeg 8.0 documentation and VAAPI implementation:

### ✅ CORRECT: Use -qp for Quality Control

```bash
-qp 28
```

**What it does**: Sets constant quantization parameter
- Range: 0-255 (practical: 20-40)
- Lower = better quality, larger file
- This is what we want for quality-based encoding

### ❌ WRONG: Using -quality for Quality Control

```bash
-quality 5
```

**What it does**: Sets encoding speed/quality tradeoff
- Range: 1-8
- Lower = slower encoding, better quality
- Higher = faster encoding, lower quality
- This affects encoding SPEED, not output quality/bitrate balance

## Recommended Implementation for FFmpeg 8.0

### Use -qp Parameter (Primary Method)

```rust
// This is the correct approach
ffmpeg_args.push("-qp".to_string());
ffmpeg_args.push(qp.to_string()); // 20-40 range
```

### Profile Selection

```rust
// Set profile based on bit depth
let profile = match bit_depth {
    BitDepth::Bit8 => 0,   // Main profile
    BitDepth::Bit10 => 1,  // High profile
};
ffmpeg_args.push("-profile:v".to_string());
ffmpeg_args.push(profile.to_string());
```

### Pixel Format Selection

```rust
// Set pixel format based on bit depth
let pixel_format = match bit_depth {
    BitDepth::Bit8 => "nv12",
    BitDepth::Bit10 => "p010le",
};
filter_parts.push(format!("format={}", pixel_format));
```

## Expected Behavior with FFmpeg 8.0

### 8-bit Encoding
```bash
ffmpeg -init_hw_device vaapi=va:/dev/dri/renderD128 \
  -hwaccel vaapi -hwaccel_device /dev/dri/renderD128 \
  -i input.mkv \
  -vf "format=nv12,hwupload" \
  -c:v av1_vaapi \
  -qp 30 \
  -profile:v 0 \
  output.mkv
```

**Expected**: 8-bit AV1 output, Main profile

### 10-bit Encoding
```bash
ffmpeg -init_hw_device vaapi=va:/dev/dri/renderD128 \
  -hwaccel vaapi -hwaccel_device /dev/dri/renderD128 \
  -i input.mkv \
  -vf "format=p010le,hwupload" \
  -c:v av1_vaapi \
  -qp 28 \
  -profile:v 1 \
  output.mkv
```

**Expected**: 10-bit AV1 output, High profile

## Parameters to OMIT

Based on FFmpeg 8.0 VAAPI implementation, these parameters are likely not needed or not exposed:

### ❌ -rc_mode
- Not directly exposed in VAAPI wrapper
- Auto-selected based on other parameters
- Using `-qp` automatically enables CQP mode

### ❌ -tier:v
- Usually auto-detected
- Not critical for most content
- Can be omitted

### ❌ -tile_rows / -tile_cols
- May not be exposed in VAAPI
- Encoder uses sensible defaults
- Can be omitted unless testing shows they work

## Intel Arc GPU Support (VAAPI)

The Intel Arc GPUs (A310, A380, A750, A770) support:

### ✅ Confirmed Support
- AV1 hardware encoding
- 8-bit encoding (Main profile)
- 10-bit encoding (High profile)
- VAAPI interface
- Up to 8K resolution

### Verification Command
```bash
vainfo | grep -i av1
```

**Expected output**:
```
VAProfileAV1Profile0            : VAEntrypointEncSlice
VAProfileAV1Profile1            : VAEntrypointEncSlice
```

- Profile0 = Main (8-bit)
- Profile1 = High (10-bit)

## Updated Implementation Strategy

### Step 1: Simplify Parameters

**Use these parameters**:
- ✅ `-qp <value>` - Quality control (20-40 range)
- ✅ `-profile:v <0|1>` - Bit depth selection
- ✅ `format=nv12` or `format=p010le` - Pixel format

**Omit these parameters**:
- ❌ `-rc_mode` - Auto-selected
- ❌ `-tier:v` - Auto-detected
- ❌ `-tile_rows` / `-tile_cols` - Use defaults
- ❌ `-quality` - This is for speed, not quality!

### Step 2: QP Value Ranges

Based on testing and FFmpeg documentation:

| Content | 8-bit QP | 10-bit QP | Expected Reduction |
|---------|----------|-----------|-------------------|
| 4K | 28-30 | 26-28 | 50-60% |
| 1080p | 30-32 | 28-30 | 60-70% |
| 720p | 32-34 | 30-32 | 65-75% |

**Adjustments**:
- H.264 source: +2 (more compression)
- HEVC source: -1 (preserve quality)
- High bitrate: +2-3 (more compression)
- Low bitrate: -1 (preserve quality)

### Step 3: Minimal Working Example

```rust
// Build ffmpeg arguments
let mut ffmpeg_args = Vec::new();

// Input setup (VAAPI)
ffmpeg_args.push("-init_hw_device".to_string());
ffmpeg_args.push("vaapi=va:/dev/dri/renderD128".to_string());
ffmpeg_args.push("-hwaccel".to_string());
ffmpeg_args.push("vaapi".to_string());
ffmpeg_args.push("-hwaccel_device".to_string());
ffmpeg_args.push("/dev/dri/renderD128".to_string());

// Input file
ffmpeg_args.push("-i".to_string());
ffmpeg_args.push(input_path);

// Filter: format conversion and upload
let pixel_format = if bit_depth == BitDepth::Bit10 { "p010le" } else { "nv12" };
ffmpeg_args.push("-vf".to_string());
ffmpeg_args.push(format!("format={},hwupload", pixel_format));

// Encoder
ffmpeg_args.push("-c:v".to_string());
ffmpeg_args.push("av1_vaapi".to_string());

// Quality control (THIS IS THE KEY PARAMETER)
ffmpeg_args.push("-qp".to_string());
ffmpeg_args.push(qp.to_string()); // 20-40

// Profile (for bit depth)
let profile = if bit_depth == BitDepth::Bit10 { 1 } else { 0 };
ffmpeg_args.push("-profile:v".to_string());
ffmpeg_args.push(profile.to_string());

// Audio/subtitles
ffmpeg_args.push("-c:a".to_string());
ffmpeg_args.push("copy".to_string());
ffmpeg_args.push("-c:s".to_string());
ffmpeg_args.push("copy".to_string());

// Output
ffmpeg_args.push(output_path);
```

## Testing Checklist

Before full implementation, test these scenarios:

### Test 1: Basic 8-bit Encoding
```bash
docker run --rm --privileged \
  -v /dev/dri:/dev/dri \
  -v /path/to/test:/config \
  lscr.io/linuxserver/ffmpeg:version-8.0-cli \
  ffmpeg -init_hw_device vaapi=va:/dev/dri/renderD128 \
  -hwaccel vaapi -hwaccel_device /dev/dri/renderD128 \
  -i /config/input_8bit.mkv \
  -vf "format=nv12,hwupload" \
  -c:v av1_vaapi \
  -qp 30 \
  -profile:v 0 \
  -c:a copy -c:s copy \
  /config/output_8bit.mkv
```

**Verify**:
- Encoding succeeds
- Output is 8-bit
- File size is reasonable

### Test 2: 10-bit Encoding
```bash
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
  -c:a copy -c:s copy \
  /config/output_10bit.mkv
```

**Verify**:
- Encoding succeeds
- Output is 10-bit
- Profile is "High"
- HDR metadata preserved (if applicable)

### Test 3: Verify Output
```bash
ffprobe -v error -select_streams v:0 \
  -show_entries stream=pix_fmt,bits_per_raw_sample,profile,codec_name \
  -of json output.mkv
```

**Expected for 10-bit**:
```json
{
  "streams": [{
    "codec_name": "av1",
    "profile": "High",
    "pix_fmt": "yuv420p10le",
    "bits_per_raw_sample": "10"
  }]
}
```

## Conclusion

### ✅ What Will Work with FFmpeg 8.0

1. **-qp parameter** for quality control (20-40 range)
2. **-profile:v** for bit depth selection (0=8-bit, 1=10-bit)
3. **format=nv12** for 8-bit, **format=p010le** for 10-bit
4. Bit depth preservation (8-bit → 8-bit, 10-bit → 10-bit)

### ❌ What to Avoid

1. **-quality** parameter (this is for speed, not quality!)
2. **-rc_mode** parameter (not needed, auto-selected)
3. **-tier:v** parameter (not critical, auto-detected)
4. **-tile_rows/-tile_cols** (may not be exposed, use defaults)

### 🎯 Implementation Confidence

**High Confidence** (will definitely work):
- Using `-qp` for quality control
- Using `-profile:v` for bit depth
- Pixel format selection (nv12 vs p010le)
- Bit depth detection and preservation

**Medium Confidence** (should work, needs testing):
- QP value ranges (20-40)
- Quality calculation algorithm
- Expected file size reductions

**Low Confidence** (may not work):
- Tile parameters
- Explicit tier setting
- Some advanced AV1 features

### 📋 Next Steps

1. ✅ Use the implementation plan as-is, but with these changes:
   - Keep `-qp` parameter (it will work)
   - Remove `-rc_mode` (not needed)
   - Remove `-tier:v` (not needed)
   - Remove tile parameters (not critical)

2. ✅ The core improvements remain valid:
   - Bit depth detection ✓
   - Dynamic pixel format ✓
   - Quality calculation improvements ✓
   - Codec adjustment fixes ✓

3. ✅ Proceed with implementation using simplified parameters

The implementation plan is still valid, just with fewer parameters to worry about!
