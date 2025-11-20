# AV1 Encoding Flow Diagram

## Current Flow (Before Changes)

```
┌─────────────────┐
│  Input File     │
│  (8-bit/10-bit) │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  FFProbe        │
│  - Resolution   │
│  - Codec        │
│  - Bitrate      │
│  - FPS          │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  Quality Calc   │
│  - Base: 24-26  │
│  - Codec adj ❌ │ (backwards logic)
│  - Range: 20-30 │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  FFmpeg Encode  │
│  - format=nv12  │ ❌ (always 8-bit)
│  - quality=25   │ ❌ (wrong param)
│  - No profile   │ ❌ (defaults to 8-bit)
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  Output File    │
│  (ALWAYS 8-bit) │ ❌ (quality loss for 10-bit sources)
└─────────────────┘
```

## Improved Flow (After Changes)

```
┌─────────────────┐
│  Input File     │
│  (8-bit/10-bit) │
└────────┬────────┘
         │
         ▼
┌─────────────────────────────────┐
│  FFProbe (Enhanced)             │
│  - Resolution                   │
│  - Codec                        │
│  - Bitrate                      │
│  - FPS                          │
│  - pix_fmt ✓ NEW                │
│  - bits_per_raw_sample ✓ NEW   │
│  - color_transfer ✓ NEW (HDR)  │
└────────┬────────────────────────┘
         │
         ▼
┌─────────────────────────────────┐
│  Bit Depth Detection ✓ NEW     │
│  - Check bits_per_raw_sample   │
│  - Parse pix_fmt               │
│  - Check HDR metadata          │
│  → BitDepth::Bit8 or Bit10     │
└────────┬────────────────────────┘
         │
         ▼
┌─────────────────────────────────┐
│  QP Calculation (Fixed) ✓      │
│  - Base QP by resolution       │
│  - Bit depth consideration ✓   │
│  - Codec adj (CORRECTED) ✓     │
│  - Bitrate efficiency ✓        │
│  - Range: 20-40 ✓              │
└────────┬────────────────────────┘
         │
         ▼
┌─────────────────────────────────┐
│  Encoding Params ✓ NEW         │
│  ┌───────────────────────────┐ │
│  │ 8-bit Path                │ │
│  │ - pixel_format: nv12      │ │
│  │ - av1_profile: 0 (Main)   │ │
│  │ - qp: 28-34               │ │
│  └───────────────────────────┘ │
│  ┌───────────────────────────┐ │
│  │ 10-bit Path ✓             │ │
│  │ - pixel_format: p010le ✓  │ │
│  │ - av1_profile: 1 (High) ✓ │ │
│  │ - qp: 26-32 ✓             │ │
│  └───────────────────────────┘ │
└────────┬────────────────────────┘
         │
         ▼
┌─────────────────────────────────┐
│  FFmpeg Encode (Enhanced) ✓    │
│  - format={dynamic} ✓          │
│  - rc_mode CQP ✓ NEW           │
│  - qp={calculated} ✓           │
│  - profile:v {0|1} ✓ NEW       │
│  - tier:v 0 ✓ NEW              │
│  - tile_rows 1 ✓ NEW           │
│  - tile_cols 2 ✓ NEW           │
└────────┬────────────────────────┘
         │
         ▼
┌─────────────────────────────────┐
│  Output File                    │
│  - 8-bit → 8-bit AV1 ✓         │
│  - 10-bit → 10-bit AV1 ✓       │
│  - HDR preserved ✓             │
└─────────────────────────────────┘
```

## Bit Depth Detection Logic

```
┌─────────────────┐
│  Video Stream   │
└────────┬────────┘
         │
         ▼
    ┌────────────────────────┐
    │ bits_per_raw_sample?   │
    └─┬──────────────────┬───┘
      │ YES              │ NO
      ▼                  ▼
   ┌──────┐         ┌──────────┐
   │ "10" │         │ pix_fmt? │
   └──┬───┘         └─┬────┬───┘
      │               │    │
      │ YES           │    │ NO
      ▼               ▼    ▼
   ┌──────────┐   ┌─────────┐  ┌──────────────┐
   │ Bit10    │   │ "10" in │  │ HDR metadata?│
   └──────────┘   │ name?   │  └─┬────────┬───┘
                  └─┬───┬───┘    │        │
                    │   │        │ YES    │ NO
                    │   │ NO     ▼        ▼
                    │   ▼     ┌──────┐ ┌──────┐
                    │ ┌──────┐│Bit10 ││ Bit8 │
                    │ │ Bit8 │└──────┘└──────┘
                    │ └──────┘
                    ▼
                 ┌──────┐
                 │Bit10 │
                 └──────┘
```

## Quality (QP) Calculation Flow

```
┌─────────────────────────────────┐
│  Input Metadata                 │
│  - Resolution: 1920x1080        │
│  - Codec: h264                  │
│  - Bitrate: 10 Mbps             │
│  - FPS: 23.976                  │
│  - Bit Depth: 8-bit             │
└────────┬────────────────────────┘
         │
         ▼
┌─────────────────────────────────┐
│  Step 1: Base QP                │
│  1080p + 8-bit → QP = 32        │
└────────┬────────────────────────┘
         │
         ▼
┌─────────────────────────────────┐
│  Step 2: Codec Adjustment       │
│  H.264 → QP += 2                │
│  QP = 34                        │
└────────┬────────────────────────┘
         │
         ▼
┌─────────────────────────────────┐
│  Step 3: Bitrate Efficiency     │
│  bpppf = 10M / (1920*1080*24)   │
│       = 0.20 (medium)           │
│  → QP += 1                      │
│  QP = 35                        │
└────────┬────────────────────────┘
         │
         ▼
┌─────────────────────────────────┐
│  Step 4: Frame Rate             │
│  23.976 fps (normal)            │
│  → No adjustment                │
│  QP = 35                        │
└────────┬────────────────────────┘
         │
         ▼
┌─────────────────────────────────┐
│  Step 5: Clamp to Range         │
│  QP = max(20, min(40, 35))      │
│  QP = 35 ✓                      │
└────────┬────────────────────────┘
         │
         ▼
┌─────────────────────────────────┐
│  Final QP: 35                   │
│  Expected reduction: ~65%       │
└─────────────────────────────────┘
```

## Encoding Parameter Selection

```
                    ┌─────────────────┐
                    │  Bit Depth      │
                    └────────┬────────┘
                             │
                    ┌────────┴────────┐
                    │                 │
                    ▼                 ▼
            ┌───────────────┐  ┌───────────────┐
            │   8-bit       │  │   10-bit      │
            └───────┬───────┘  └───────┬───────┘
                    │                  │
        ┌───────────┼──────────┐      │
        │           │          │      │
        ▼           ▼          ▼      ▼
    ┌──────┐  ┌─────────┐  ┌──────┐ ┌─────────┐
    │format│  │profile:v│  │format│ │profile:v│
    │=nv12 │  │   = 0   │  │=p010 │ │   = 1   │
    └──────┘  └─────────┘  └──────┘ └─────────┘
        │           │          │      │
        └───────────┼──────────┘      │
                    │                 │
                    └────────┬────────┘
                             │
                    ┌────────▼────────┐
                    │  Common Params  │
                    │  - rc_mode CQP  │
                    │  - qp {value}   │
                    │  - tier:v 0     │
                    │  - tile_rows 1  │
                    │  - tile_cols 2  │
                    └─────────────────┘
```

## Data Flow Through Code

```
main.rs
  │
  ├─► probe_file()
  │     └─► FFProbeData
  │           └─► FFProbeStream
  │                 ├─► pix_fmt ✓
  │                 ├─► bits_per_raw_sample ✓
  │                 └─► color_* ✓
  │
  ├─► detect_bit_depth()
  │     └─► BitDepth enum ✓
  │
  ├─► determine_encoding_params()
  │     ├─► calculate_optimal_qp() ✓
  │     └─► EncodingParams ✓
  │           ├─► bit_depth
  │           ├─► pixel_format
  │           ├─► av1_profile
  │           ├─► qp
  │           └─► is_hdr
  │
  └─► run_av1_vaapi_job()
        ├─► Build filter chain (dynamic format) ✓
        ├─► Add rate control params ✓
        ├─► Add profile/tier ✓
        ├─► Add tiles ✓
        └─► Execute ffmpeg
              └─► Output file (correct bit depth) ✓
```

## File Modification Map

```
crates/
├── daemon/
│   └── src/
│       ├── ffprobe.rs ⚙️ MODIFY
│       │   ├── Add fields to FFProbeStream
│       │   ├── Add BitDepth enum
│       │   ├── Add detect_bit_depth()
│       │   └── Add is_hdr_content()
│       │
│       ├── job.rs ⚙️ MODIFY
│       │   └── Add bit depth tracking fields
│       │
│       └── ffmpeg_docker.rs ⚙️ MODIFY (MAJOR)
│           ├── Add EncodingParams struct
│           ├── Add determine_encoding_params()
│           ├── Fix calculate_optimal_qp()
│           └── Update run_av1_vaapi_job()
│
└── cli-daemon/
    └── src/
        └── main.rs ⚙️ MODIFY
            ├── Extract bit depth
            ├── Determine encoding params
            └── Update job tracking
```

## Testing Flow

```
┌─────────────────┐
│  Test File      │
│  (Known props)  │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  Run Encoding   │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  Check Output   │
│  with ffprobe   │
└────────┬────────┘
         │
         ├─► Bit depth correct? ✓
         ├─► Profile correct? ✓
         ├─► HDR preserved? ✓
         ├─► File size OK? ✓
         └─► Quality good? ✓
```

## Success Criteria Visualization

```
Before:
  8-bit source  ──┐
                  ├──► 8-bit output ✓
 10-bit source  ──┘    (quality loss ❌)

After:
  8-bit source  ────► 8-bit output ✓
 10-bit source  ────► 10-bit output ✓
```

---

This diagram shows the complete flow from input to output, highlighting all the new components and improvements.
