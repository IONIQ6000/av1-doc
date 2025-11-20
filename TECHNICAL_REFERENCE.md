# Technical Reference - AV1 VAAPI Encoding

## Intel VAAPI AV1 Encoder Parameters

### Rate Control Modes

```bash
-rc_mode <mode>
```

- **CQP** (Constant Quantization Parameter): Quality-based encoding
  - Best for: Archival, quality-focused encoding
  - Predictable quality, variable bitrate
  - Use with `-qp` parameter

- **CBR** (Constant Bitrate): Fixed bitrate
  - Best for: Streaming, bandwidth-limited scenarios
  - Use with `-b:v` parameter

- **VBR** (Variable Bitrate): Variable bitrate with target
  - Best for: Balanced quality/size
  - Use with `-b:v` and `-maxrate` parameters

### Quantization Parameter (QP)

```bash
-qp <value>
```

Range: 0-255 (lower = better quality, larger file)

**Practical Ranges**:
- **20-24**: Very high quality, minimal compression (~40-50% reduction)
- **25-28**: High quality, good compression (~50-60% reduction)
- **29-32**: Balanced quality/compression (~60-70% reduction)
- **33-36**: Higher compression (~70-75% reduction)
- **37-40**: Aggressive compression (~75-80% reduction)
- **41+**: Visible quality loss (not recommended)

**Recommendations by Content Type**:
- 4K content: QP 26-30
- 1080p content: QP 28-32
- 720p content: QP 30-34
- Animation: QP -2 (less compression, preserve lines)
- Grainy film: QP +2 (grain compresses poorly)

### AV1 Profile

```bash
-profile:v <profile>
```

- **0** (Main): 8-bit, 4:2:0 chroma subsampling
  - Use for: Standard 8-bit content
  - Pixel format: nv12

- **1** (High): 10-bit, 4:2:0 chroma subsampling
  - Use for: 10-bit content, HDR
  - Pixel format: p010le

- **2** (Professional): 12-bit, 4:2:2/4:4:4 chroma subsampling
  - Use for: Professional workflows (rarely needed)
  - Not commonly supported by VAAPI

### AV1 Tier

```bash
-tier:v <tier>
```

- **0** (Main): Up to 4K @ 60fps, bitrates up to ~30 Mbps
  - Use for: 99% of content
  
- **1** (High): Higher bitrates and resolutions
  - Use for: 8K content or very high bitrate 4K

### Tile Configuration

```bash
-tile_rows <rows>
-tile_cols <cols>
```

Tiles enable parallel encoding. More tiles = faster encoding but slightly lower compression efficiency.

**Recommendations**:
- **1080p and below**: 1 row, 1 col (no tiling needed)
- **4K**: 1 row, 2 cols (good balance)
- **8K**: 2 rows, 4 cols (maximum parallelization)

**Note**: Intel Arc GPUs benefit from tiling for 4K+ content.

### Level

```bash
-level <level>
```

AV1 levels define maximum resolution, bitrate, and decode complexity.

Common levels:
- **4.0**: 1080p @ 60fps
- **5.0**: 4K @ 30fps
- **5.1**: 4K @ 60fps
- **6.0**: 8K @ 30fps

Usually auto-detected, rarely needs manual setting.

## Pixel Formats

### 8-bit Formats

**nv12** (Recommended for VAAPI)
- 4:2:0 chroma subsampling
- 8 bits per component
- Interleaved UV plane
- Best hardware support

**yuv420p**
- 4:2:0 chroma subsampling
- 8 bits per component
- Planar format
- Software encoding

### 10-bit Formats

**p010le** (Recommended for VAAPI)
- 4:2:0 chroma subsampling
- 10 bits per component (stored in 16-bit)
- Interleaved UV plane
- Best hardware support for 10-bit

**yuv420p10le**
- 4:2:0 chroma subsampling
- 10 bits per component
- Planar format
- Software encoding

### Format Conversion

```bash
# 8-bit pipeline
format=nv12,hwupload

# 10-bit pipeline
format=p010le,hwupload
```

## Bit Depth Detection

### Method 1: bits_per_raw_sample (Most Reliable)

```json
{
  "streams": [{
    "bits_per_raw_sample": "10"
  }]
}
```

If value is "10" or 10 → 10-bit
If value is "8" or 8 → 8-bit

### Method 2: Pixel Format Parsing

Look for "10" in pixel format name:
- `yuv420p10le` → 10-bit
- `yuv420p` → 8-bit
- `p010le` → 10-bit
- `nv12` → 8-bit

### Method 3: HDR Metadata (Implies 10-bit)

Check color transfer characteristics:
- `smpte2084` (PQ) → HDR, 10-bit
- `arib-std-b67` (HLG) → HDR, 10-bit
- `bt709` → SDR, likely 8-bit
- `bt2020-10` → 10-bit

### Method 4: Color Primaries

- `bt2020` → Often 10-bit (UHD Blu-ray)
- `bt709` → Often 8-bit (HD Blu-ray, web)

## HDR Detection

HDR content requires 10-bit encoding to preserve quality.

**HDR Indicators**:
1. `color_transfer`: "smpte2084" or "arib-std-b67"
2. `color_primaries`: "bt2020"
3. `color_space`: "bt2020nc" or "bt2020c"
4. Presence of HDR metadata (mastering display, content light level)

**HDR Types**:
- **HDR10**: Static metadata, PQ transfer (smpte2084)
- **HDR10+**: Dynamic metadata, PQ transfer
- **Dolby Vision**: Proprietary, dual-layer
- **HLG**: Hybrid Log-Gamma (arib-std-b67), broadcast-friendly

## Quality Calculation Algorithm

### Base QP by Resolution and Bit Depth

| Resolution | 8-bit Base QP | 10-bit Base QP |
|------------|---------------|----------------|
| 4K (2160p) | 28 | 26 |
| 1440p | 30 | 28 |
| 1080p | 32 | 30 |
| 720p | 34 | 32 |
| 480p | 36 | 34 |

### Source Codec Adjustment

| Codec | Adjustment | Reason |
|-------|------------|--------|
| H.264/AVC | +2 | Inefficient, can compress more |
| HEVC/H.265 | -1 | Already efficient, preserve quality |
| VP9 | -1 | Already efficient |
| AV1 | 0 | Already optimal |
| MPEG-2 | +3 | Very inefficient |

### Bitrate Efficiency Adjustment

Calculate bits per pixel per frame (bpppf):
```
bpppf = bitrate_bps / (width × height × fps)
```

| bpppf Range | Adjustment | Description |
|-------------|------------|-------------|
| > 0.6 | +3 | Very high bitrate, compress aggressively |
| 0.4 - 0.6 | +2 | High bitrate, good compression room |
| 0.2 - 0.4 | +1 | Medium bitrate, moderate compression |
| 0.1 - 0.2 | 0 | Balanced, baseline |
| < 0.1 | -1 | Low bitrate, preserve quality |

### Frame Rate Adjustment

| FPS Range | Adjustment | Reason |
|-----------|------------|--------|
| > 50 | -1 | High motion, preserve detail |
| 24-50 | 0 | Standard, no adjustment |
| < 24 | +1 | Low motion, can compress more |

### Content Type Adjustment (Future)

| Content Type | Adjustment | Reason |
|--------------|------------|--------|
| Animation | -2 | Preserve sharp lines |
| Grainy film | +2 | Grain compresses poorly |
| Screen capture | -2 | Text needs clarity |
| Sports | -1 | Fast motion needs quality |

### Final Clamping

- Minimum QP: 20 (prevents excessive file sizes)
- Maximum QP: 40 (prevents quality loss)

## Expected File Size Reductions

### By Source Type

| Source | Expected Reduction |
|--------|-------------------|
| 8-bit H.264 high bitrate | 60-70% |
| 8-bit H.264 medium bitrate | 50-60% |
| 8-bit H.264 low bitrate | 40-50% |
| 10-bit HEVC high bitrate | 45-55% |
| 10-bit HEVC medium bitrate | 35-45% |
| 10-bit HEVC low bitrate | 25-35% |
| VP9 | 20-30% |
| MPEG-2 | 70-80% |

### By QP Value

| QP Range | Expected Reduction |
|----------|-------------------|
| 20-24 | 40-50% |
| 25-28 | 50-60% |
| 29-32 | 60-70% |
| 33-36 | 70-75% |
| 37-40 | 75-80% |

## FFmpeg Command Examples

### 8-bit Encoding

```bash
ffmpeg -init_hw_device vaapi=va:/dev/dri/renderD128 \
  -hwaccel vaapi -hwaccel_device /dev/dri/renderD128 \
  -i input.mkv \
  -vf "format=nv12,hwupload" \
  -c:v av1_vaapi \
  -rc_mode CQP \
  -qp 30 \
  -profile:v 0 \
  -tier:v 0 \
  -tile_rows 1 -tile_cols 2 \
  -c:a copy -c:s copy \
  output.mkv
```

### 10-bit Encoding

```bash
ffmpeg -init_hw_device vaapi=va:/dev/dri/renderD128 \
  -hwaccel vaapi -hwaccel_device /dev/dri/renderD128 \
  -i input.mkv \
  -vf "format=p010le,hwupload" \
  -c:v av1_vaapi \
  -rc_mode CQP \
  -qp 28 \
  -profile:v 1 \
  -tier:v 0 \
  -tile_rows 1 -tile_cols 2 \
  -c:a copy -c:s copy \
  output.mkv
```

### Verify Output

```bash
# Check bit depth
ffprobe -v error -select_streams v:0 \
  -show_entries stream=pix_fmt,bits_per_raw_sample,profile \
  -of json output.mkv

# Check HDR metadata
ffprobe -v error -select_streams v:0 \
  -show_entries stream=color_space,color_transfer,color_primaries \
  -of json output.mkv
```

## Common Issues and Solutions

### Issue: Output is 8-bit when source is 10-bit

**Cause**: Using nv12 format instead of p010le
**Solution**: Use `format=p010le` and `-profile:v 1`

### Issue: HDR metadata lost

**Cause**: Not copying color metadata
**Solution**: Ensure ffmpeg copies color_space, color_transfer, color_primaries

### Issue: File size too large

**Cause**: QP too low
**Solution**: Increase QP value (higher = more compression)

### Issue: Quality loss/banding

**Cause**: QP too high or 10-bit source encoded as 8-bit
**Solution**: Lower QP or ensure 10-bit encoding for 10-bit sources

### Issue: Encoding fails with "unsupported profile"

**Cause**: Hardware doesn't support 10-bit
**Solution**: Check GPU capabilities, fall back to 8-bit if needed

## Intel Arc GPU Capabilities

### AV1 Encoding Support

- **Arc A310/A380/A580/A750/A770**: Full AV1 encode support
  - 8-bit: ✅ Supported
  - 10-bit: ✅ Supported
  - HDR: ✅ Supported
  - Max resolution: 8K

### Performance Expectations

- **1080p**: ~200-400 fps (real-time)
- **4K**: ~60-120 fps (real-time)
- **8K**: ~15-30 fps

### Power Consumption

- Idle: ~5-10W
- 1080p encode: ~15-25W
- 4K encode: ~30-50W

## References

- [FFmpeg VAAPI Documentation](https://trac.ffmpeg.org/wiki/Hardware/VAAPI)
- [AV1 Specification](https://aomediacodec.github.io/av1-spec/)
- [Intel Media SDK](https://github.com/Intel-Media-SDK/MediaSDK)
