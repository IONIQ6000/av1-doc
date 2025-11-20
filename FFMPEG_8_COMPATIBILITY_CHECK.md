# FFmpeg 8.0 av1_vaapi Compatibility Check

## Docker Image
`lscr.io/linuxserver/ffmpeg:version-8.0-cli`

## Critical: Verify Encoder Capabilities

Before implementing, we need to verify what parameters the av1_vaapi encoder actually supports in FFmpeg 8.0.

### Test Command

Run this to check available parameters:

```bash
docker run --rm lscr.io/linuxserver/ffmpeg:version-8.0-cli \
  ffmpeg -h encoder=av1_vaapi
```

### Expected Output Analysis

Look for these sections:

#### 1. Rate Control Parameters

Check if these are supported:
- `-qp <int>` - Quantization parameter
- `-quality <int>` - Quality/speed tradeoff
- `-rc_mode <int>` - Rate control mode
- `-b:v <bitrate>` - Target bitrate

#### 2. Profile/Level Parameters

Check if these are supported:
- `-profile:v <int>` - AV1 profile (0=Main, 1=High)
- `-tier:v <int>` - AV1 tier
- `-level <int>` - AV1 level

#### 3. Tile Parameters

Check if these are supported:
- `-tile_rows <int>` - Number of tile rows
- `-tile_cols <int>` - Number of tile columns

## Known FFmpeg 8.0 av1_vaapi Behavior

Based on FFmpeg 8.0 documentation, the av1_vaapi encoder typically supports:

### ✅ Likely Supported
- `-b:v` - Bitrate (VBR mode)
- `-maxrate` - Maximum bitrate
- `-qp` - Constant QP mode
- `-quality` - Quality/speed tradeoff (1-8 range, NOT 0-255)
- `-profile:v` - Profile selection

### ⚠️ May Not Be Supported
- `-rc_mode` - May not be exposed as a parameter
- `-tier:v` - May not be exposed
- `-tile_rows` / `-tile_cols` - May not be exposed in VAAPI

### 🔍 Need to Verify
- Actual QP range (may be 0-255 or 1-63)
- Whether `-quality` or `-qp` is the correct parameter
- Profile support for 10-bit

## Alternative Approach: Use What Works

If some parameters aren't supported, we can adapt:

### Option 1: Use -qp (Preferred)
```bash
-qp 28
```
This should work for constant quality encoding.

### Option 2: Use -quality (Fallback)
```bash
-quality 5
```
Range is typically 1-8, where:
- 1 = Best quality, slowest
- 4 = Balanced
- 8 = Lowest quality, fastest

**Note**: This is speed/quality tradeoff, not bitrate/quality!

### Option 3: Use Bitrate Mode
```bash
-b:v 5M -maxrate 8M
```
Use target bitrate instead of quality-based encoding.

## Recommended Testing Sequence

### Test 1: Check Basic Encoding
```bash
docker run --rm --privileged \
  -v /dev/dri:/dev/dri \
  -v /path/to/test:/config \
  lscr.io/linuxserver/ffmpeg:version-8.0-cli \
  ffmpeg -init_hw_device vaapi=va:/dev/dri/renderD128 \
  -hwaccel vaapi -hwaccel_device /dev/dri/renderD128 \
  -i /config/input.mkv \
  -vf "format=nv12,hwupload" \
  -c:v av1_vaapi \
  -qp 28 \
  /config/output.mkv
```

### Test 2: Check 10-bit Support
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
  -profile:v 1 \
  -qp 28 \
  /config/output.mkv
```

### Test 3: Check Profile Support
```bash
# Try with explicit profile
-profile:v 1

# If that fails, try without profile and check output
```

### Test 4: Verify Output
```bash
docker run --rm \
  -v /path/to/test:/config \
  lscr.io/linuxserver/ffmpeg:version-8.0-cli \
  ffprobe -v error -select_streams v:0 \
  -show_entries stream=pix_fmt,bits_per_raw_sample,profile \
  -of json /config/output.mkv
```

## Potential Issues and Workarounds

### Issue 1: -qp Not Supported

**Symptom**: Error like "Option qp not found"

**Workaround**: Use `-quality` parameter instead
```rust
// Instead of -qp
ffmpeg_args.push("-quality".to_string());
ffmpeg_args.push("5".to_string()); // Range 1-8
```

**Quality Mapping**:
- QP 20-24 → quality 2-3 (very high quality)
- QP 25-28 → quality 4-5 (high quality)
- QP 29-32 → quality 5-6 (balanced)
- QP 33-36 → quality 6-7 (more compression)
- QP 37-40 → quality 7-8 (high compression)

### Issue 2: -profile:v Not Supported

**Symptom**: Profile parameter ignored or error

**Workaround**: Let encoder auto-detect from pixel format
- p010le input → Should automatically use 10-bit profile
- nv12 input → Should automatically use 8-bit profile

### Issue 3: -rc_mode Not Supported

**Symptom**: Error like "Option rc_mode not found"

**Workaround**: Omit the parameter, encoder will use appropriate mode based on other parameters
- If `-qp` is set → Uses constant QP mode
- If `-b:v` is set → Uses VBR mode

### Issue 4: Tile Parameters Not Supported

**Symptom**: Error like "Option tile_rows not found"

**Workaround**: Omit tile parameters, encoder will use defaults

### Issue 5: 10-bit Encoding Not Working

**Symptom**: Output is 8-bit even with p010le input

**Possible Causes**:
1. GPU doesn't support 10-bit AV1 encoding
2. FFmpeg build doesn't support 10-bit VAAPI
3. Profile not set correctly

**Workaround**: 
- Check GPU capabilities: `vainfo | grep AV1`
- If no 10-bit support, fall back to 8-bit for all content
- Add detection and warning in code

## Updated Implementation Strategy

Based on potential limitations, here's the adapted approach:

### Step 1: Test Current Docker Image

Before implementing, run these tests:

```bash
# Test 1: Check encoder help
docker run --rm lscr.io/linuxserver/ffmpeg:version-8.0-cli \
  ffmpeg -h encoder=av1_vaapi > av1_vaapi_help.txt

# Test 2: Try encoding with -qp
# (use actual test file)

# Test 3: Try encoding with -quality
# (use actual test file)

# Test 4: Try 10-bit encoding
# (use actual 10-bit test file)
```

### Step 2: Adapt Implementation Based on Results

Create a compatibility layer in code:

```rust
pub struct EncoderCapabilities {
    pub supports_qp: bool,
    pub supports_quality: bool,
    pub supports_profile: bool,
    pub supports_10bit: bool,
    pub supports_tiles: bool,
}

// Detect capabilities once at startup
pub fn detect_encoder_capabilities() -> EncoderCapabilities {
    // Run test encodes or parse ffmpeg -h output
    // Return what's actually supported
}
```

### Step 3: Use Fallback Parameters

```rust
// Preferred: Use -qp if supported
if capabilities.supports_qp {
    ffmpeg_args.push("-qp".to_string());
    ffmpeg_args.push(qp.to_string());
} else if capabilities.supports_quality {
    // Fallback: Map QP to quality range
    let quality = map_qp_to_quality(qp); // 1-8
    ffmpeg_args.push("-quality".to_string());
    ffmpeg_args.push(quality.to_string());
} else {
    // Last resort: Use bitrate mode
    let bitrate = calculate_target_bitrate(meta);
    ffmpeg_args.push("-b:v".to_string());
    ffmpeg_args.push(format!("{}M", bitrate));
}
```

## Action Items Before Implementation

- [ ] Run `docker run --rm lscr.io/linuxserver/ffmpeg:version-8.0-cli ffmpeg -h encoder=av1_vaapi`
- [ ] Save output to file for analysis
- [ ] Test basic 8-bit encoding with `-qp`
- [ ] Test basic 8-bit encoding with `-quality`
- [ ] Test 10-bit encoding with `p010le`
- [ ] Test profile parameter
- [ ] Test tile parameters
- [ ] Document what actually works
- [ ] Update implementation plan based on findings

## Expected Reality Check

**Most Likely Scenario**:
- ✅ `-quality` parameter works (1-8 range)
- ✅ Pixel format selection works (nv12 vs p010le)
- ⚠️ `-qp` may or may not work
- ⚠️ `-profile:v` may be auto-detected
- ❌ `-rc_mode` probably not exposed
- ❌ Tile parameters probably not exposed

**Recommended Approach**:
1. Use `-quality` parameter (proven to work)
2. Map our QP calculations to quality range (1-8)
3. Let profile auto-detect from pixel format
4. Omit tile and rc_mode parameters
5. Focus on bit depth preservation (the main goal)

## Quality Mapping Function

If we need to use `-quality` instead of `-qp`:

```rust
/// Map QP value (20-40) to FFmpeg quality parameter (1-8)
/// Lower quality number = better quality (same as QP)
fn map_qp_to_quality(qp: i32) -> i32 {
    // QP 20-40 → quality 1-8
    // Linear mapping
    let quality = ((qp - 20) as f64 / 20.0 * 7.0) + 1.0;
    quality.round() as i32
}

// Examples:
// QP 20 → quality 1 (best)
// QP 25 → quality 2.75 → 3
// QP 30 → quality 4.5 → 5 (balanced)
// QP 35 → quality 6.25 → 6
// QP 40 → quality 8 (most compression)
```

## Next Steps

1. **FIRST**: Run the compatibility tests above
2. **THEN**: Update implementation plan based on actual capabilities
3. **FINALLY**: Proceed with implementation using supported parameters

---

**DO NOT proceed with implementation until we verify what parameters actually work!**
