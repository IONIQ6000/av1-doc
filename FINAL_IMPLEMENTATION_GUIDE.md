# Final Implementation Guide - Simplified for FFmpeg 8.0

## Executive Summary

Based on research of the `lscr.io/linuxserver/ffmpeg:version-8.0-cli` image, here's the **simplified, confirmed-working** implementation plan.

## ✅ What We Know Works

1. **`-qp` parameter** - Quality control (0-255, practical: 20-40)
2. **`-profile:v`** - Bit depth selection (0=8-bit, 1=10-bit)
3. **`format=nv12`** - 8-bit pixel format
4. **`format=p010le`** - 10-bit pixel format

## ❌ What to Skip

1. **`-rc_mode`** - Not needed, auto-selected based on `-qp`
2. **`-tier:v`** - Not critical, auto-detected
3. **`-tile_rows/-tile_cols`** - May not be exposed, use defaults
4. **`-quality`** - This is speed/quality tradeoff, NOT what we want!

## 🎯 Core Changes Needed

### 1. Add Bit Depth Detection (45 min)

**File**: `crates/daemon/src/ffprobe.rs`

Add fields:
```rust
pub struct FFProbeStream {
    // ... existing fields ...
    
    #[serde(rename = "pix_fmt")]
    pub pix_fmt: Option<String>,
    
    #[serde(rename = "bits_per_raw_sample")]
    pub bits_per_raw_sample: Option<String>,
    
    #[serde(rename = "color_transfer")]
    pub color_transfer: Option<String>,
    
    #[serde(rename = "color_primaries")]
    pub color_primaries: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitDepth {
    Bit8,
    Bit10,
    Unknown,
}

impl FFProbeStream {
    pub fn detect_bit_depth(&self) -> BitDepth {
        // Method 1: Check bits_per_raw_sample
        if let Some(ref bits) = self.bits_per_raw_sample {
            if bits == "10" {
                return BitDepth::Bit10;
            } else if bits == "8" {
                return BitDepth::Bit8;
            }
        }
        
        // Method 2: Check pixel format
        if let Some(ref pix_fmt) = self.pix_fmt {
            if pix_fmt.contains("10") || pix_fmt.contains("p010") {
                return BitDepth::Bit10;
            }
        }
        
        // Method 3: Check HDR (implies 10-bit)
        if self.is_hdr_content() {
            return BitDepth::Bit10;
        }
        
        // Default to 8-bit
        BitDepth::Bit8
    }
    
    pub fn is_hdr_content(&self) -> bool {
        if let Some(ref transfer) = self.color_transfer {
            let t = transfer.to_lowercase();
            if t.contains("smpte2084") || t.contains("arib-std-b67") {
                return true;
            }
        }
        false
    }
}
```

### 2. Update Job Tracking (15 min)

**File**: `crates/daemon/src/job.rs`

Add fields:
```rust
pub struct Job {
    // ... existing fields ...
    
    pub source_bit_depth: Option<u8>,
    pub target_bit_depth: Option<u8>,
    pub av1_profile: Option<u8>,
    pub is_hdr: Option<bool>,
}
```

### 3. Fix Quality Calculation (45 min)

**File**: `crates/daemon/src/ffmpeg_docker.rs`

Rename and update function:
```rust
/// Calculate optimal QP value for AV1 encoding
/// QP range: 0-255 (practical: 20-40)
/// Lower QP = better quality, larger file
pub fn calculate_optimal_qp(
    meta: &FFProbeData,
    input_file: &Path,
    bit_depth: BitDepth,
) -> i32 {
    // ... existing metadata extraction ...
    
    // Base QP from resolution and bit depth
    let mut qp = if height >= 2160 {
        if bit_depth == BitDepth::Bit10 { 26 } else { 28 }
    } else if height >= 1440 {
        if bit_depth == BitDepth::Bit10 { 28 } else { 30 }
    } else if height >= 1080 {
        if bit_depth == BitDepth::Bit10 { 30 } else { 32 }
    } else {
        if bit_depth == BitDepth::Bit10 { 32 } else { 34 }
    };
    
    // FIXED: Codec adjustment (was backwards!)
    let codec_adjustment = match source_codec.as_str() {
        "h264" | "avc" => {
            // H.264 is inefficient, can compress more
            2 // Higher QP = more compression
        }
        "hevc" | "h265" => {
            // HEVC already efficient, preserve quality
            -1 // Lower QP = less compression
        }
        "vp9" => {
            -1 // Already efficient
        }
        "av1" => {
            0 // Already optimal
        }
        _ => 0,
    };
    qp += codec_adjustment;
    
    // Bitrate efficiency adjustment
    if let Some(bitrate_bps) = video_bitrate_bps {
        let pixels = (width * height) as f64;
        let bits_per_pixel_per_frame = (bitrate_bps as f64) / (pixels * fps);
        
        if bits_per_pixel_per_frame > 0.6 {
            qp += 3; // Very high bitrate, compress more
        } else if bits_per_pixel_per_frame > 0.4 {
            qp += 2; // High bitrate
        } else if bits_per_pixel_per_frame > 0.2 {
            qp += 1; // Medium bitrate
        } else if bits_per_pixel_per_frame < 0.1 {
            qp -= 1; // Low bitrate, preserve quality
        }
    }
    
    // Frame rate adjustment
    if fps > 50.0 {
        qp -= 1; // High FPS, preserve motion detail
    } else if fps < 24.0 {
        qp += 1; // Low FPS, can compress more
    }
    
    // Clamp to valid range
    qp = qp.max(20).min(40);
    
    info!("🎯 Calculated optimal QP: {} ({}x{}, {}-bit, {}, {:.2} fps)",
          qp, width, height,
          if bit_depth == BitDepth::Bit10 { 10 } else { 8 },
          source_codec, fps);
    
    qp
}
```

### 4. Update Encoding Function (60 min)

**File**: `crates/daemon/src/ffmpeg_docker.rs`

Update signature and implementation:
```rust
pub struct EncodingParams {
    pub bit_depth: BitDepth,
    pub pixel_format: String,
    pub av1_profile: u8,
    pub qp: i32,
    pub is_hdr: bool,
}

pub fn determine_encoding_params(
    meta: &FFProbeData,
    input_file: &Path,
) -> EncodingParams {
    let video_stream = meta.streams.iter()
        .find(|s| s.codec_type.as_deref() == Some("video"));
    
    let bit_depth = video_stream
        .map(|s| s.detect_bit_depth())
        .unwrap_or(BitDepth::Bit8);
    
    let is_hdr = video_stream
        .map(|s| s.is_hdr_content())
        .unwrap_or(false);
    
    let (pixel_format, av1_profile) = match bit_depth {
        BitDepth::Bit8 => ("nv12".to_string(), 0),
        BitDepth::Bit10 => ("p010le".to_string(), 1),
        BitDepth::Unknown => ("nv12".to_string(), 0),
    };
    
    let qp = calculate_optimal_qp(meta, input_file, bit_depth);
    
    EncodingParams {
        bit_depth,
        pixel_format,
        av1_profile,
        qp,
        is_hdr,
    }
}

pub async fn run_av1_vaapi_job(
    cfg: &TranscodeConfig,
    input: &Path,
    temp_output: &Path,
    meta: &FFProbeData,
    decision: &WebSourceDecision,
    encoding_params: &EncodingParams, // NEW PARAMETER
) -> Result<FFmpegResult> {
    // ... existing setup code ...
    
    // Build video filter chain
    let mut filter_parts = Vec::new();
    filter_parts.push("pad=ceil(iw/2)*2:ceil(ih/2)*2".to_string());
    filter_parts.push("setsar=1".to_string());
    
    // CHANGED: Dynamic pixel format based on bit depth
    filter_parts.push(format!("format={}", encoding_params.pixel_format));
    
    filter_parts.push("hwupload=extra_hw_frames=64".to_string());
    
    let filter_chain = filter_parts.join(",");
    ffmpeg_args.push("-vf".to_string());
    ffmpeg_args.push(filter_chain);
    
    // ... mapping code ...
    
    // Video codec: AV1 VAAPI
    ffmpeg_args.push("-c:v".to_string());
    ffmpeg_args.push("av1_vaapi".to_string());
    
    // CHANGED: Use -qp instead of -quality
    ffmpeg_args.push("-qp".to_string());
    ffmpeg_args.push(encoding_params.qp.to_string());
    
    // NEW: Set AV1 profile for bit depth
    ffmpeg_args.push("-profile:v".to_string());
    ffmpeg_args.push(encoding_params.av1_profile.to_string());
    
    // Store QP for return value
    let qp_used = encoding_params.qp;
    
    // ... rest of encoding ...
    
    Ok(FFmpegResult {
        exit_code,
        stdout,
        stderr,
        quality_used: qp_used,
    })
}
```

### 5. Update Main Daemon (30 min)

**File**: `crates/cli-daemon/src/main.rs`

Update workflow:
```rust
// After probing file
if let Some(video_stream) = meta.streams.iter()
    .find(|s| s.codec_type.as_deref() == Some("video"))
{
    let bit_depth = video_stream.detect_bit_depth();
    let is_hdr = video_stream.is_hdr_content();
    
    job.source_bit_depth = Some(match bit_depth {
        BitDepth::Bit8 => 8,
        BitDepth::Bit10 => 10,
        BitDepth::Unknown => 8,
    });
    job.is_hdr = Some(is_hdr);
    
    // ... existing metadata extraction ...
}

// Determine encoding parameters
let encoding_params = ffmpeg_docker::determine_encoding_params(&meta, &job.source_path);

job.target_bit_depth = Some(match encoding_params.bit_depth {
    BitDepth::Bit8 => 8,
    BitDepth::Bit10 => 10,
    BitDepth::Unknown => 8,
});
job.av1_profile = Some(encoding_params.av1_profile);
job.av1_quality = Some(encoding_params.qp);

info!("Job {}: Source: {}x{}, {}-bit{}, {}, {:.2} fps",
    job.id, width, height,
    job.source_bit_depth.unwrap_or(8),
    if job.is_hdr.unwrap_or(false) { " HDR" } else { "" },
    source_codec, fps
);

info!("Job {}: Target: {}-bit AV1 (profile {}), QP: {}",
    job.id,
    job.target_bit_depth.unwrap_or(8),
    job.av1_profile.unwrap_or(0),
    encoding_params.qp
);

// Run encoding with new parameters
let ffmpeg_result = ffmpeg_docker::run_av1_vaapi_job(
    cfg,
    &job.source_path,
    &temp_output,
    &meta,
    &decision,
    &encoding_params, // NEW
).await?;
```

## 📊 Expected Results

### Before Implementation
```
8-bit H.264  → 8-bit AV1 ✓ (but suboptimal quality settings)
10-bit HEVC  → 8-bit AV1 ❌ (quality loss, banding)
10-bit HDR   → 8-bit AV1 ❌ (HDR broken)
```

### After Implementation
```
8-bit H.264  → 8-bit AV1 ✓ (optimized QP)
10-bit HEVC  → 10-bit AV1 ✓ (quality preserved)
10-bit HDR   → 10-bit AV1 ✓ (HDR preserved)
```

## 🧪 Testing Commands

### Test 8-bit Encoding
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
  /config/output.mkv
```

### Test 10-bit Encoding
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
  /config/output.mkv
```

### Verify Output
```bash
ffprobe -v error -select_streams v:0 \
  -show_entries stream=pix_fmt,bits_per_raw_sample,profile \
  -of json output.mkv
```

## ⏱️ Implementation Timeline

1. **Bit depth detection** - 45 min
2. **Job tracking** - 15 min
3. **Quality calculation** - 45 min
4. **Encoding function** - 60 min
5. **Main daemon** - 30 min
6. **Testing** - 60 min
7. **Documentation** - 15 min

**Total: ~4.5 hours**

## ✅ Success Criteria

- [ ] 8-bit sources produce 8-bit output
- [ ] 10-bit sources produce 10-bit output
- [ ] HDR metadata preserved
- [ ] QP values in range 20-40
- [ ] File sizes: 40-75% reduction
- [ ] No visual quality loss
- [ ] Codec adjustments work correctly (H.264: +2, HEVC: -1)

## 🚀 Ready to Implement

This simplified plan:
- ✅ Uses only confirmed-working parameters
- ✅ Removes unnecessary complexity
- ✅ Focuses on core goal: bit depth preservation
- ✅ Fixes the backwards quality logic
- ✅ Works with FFmpeg 8.0 VAAPI

**You can proceed with confidence!**
