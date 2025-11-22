---
inclusion: always
---

# Spec: Software‑Only AV1 Encoding (FFmpeg ≥ 8.0) — Quality‑Max Mode

This spec tells Kiro how to refactor the current app to:
1) **Stop using Docker** for FFmpeg.
2) Use an **embedded or locally installed FFmpeg 8.0+ binary**.
3) Encode AV1 **on CPU only**, prioritizing **maximum perceptual quality** over efficiency.

FFmpeg 8.0 “Huffman” released Aug 22, 2025. The app must require **FFmpeg ≥ 8.0**. citeturn0search12turn0search3turn0search7  
FFmpeg supports CPU AV1 encoders **libsvtav1**, **libaom‑av1**, and **librav1e**. citeturn0search0turn0search25

---

## 0) Prime directive (non‑negotiable)

When converting to AV1, optimize for **maximum perceptual quality**.  
**Do not** chase compression efficiency, file size, or speed unless explicitly instructed by the user.

If there is any tradeoff between “smaller” and “better looking,” choose **better looking**.

---

## 1) Remove Docker dependency from the app

### Required code changes
- Delete/disable all Docker orchestration:
  - no `docker run`, no container lifecycle logic, no image pull/build.
- Replace with a direct subprocess call to FFmpeg:
  - use `${FFMPEG_BIN:-ffmpeg}` as the executable path.
- Add a startup check:
  - `ffmpeg -version` must parse to `>= 8.0`.
  - If version is lower or missing, app should error with a clear message.

### Config surface
Provide a single config knob:
- `FFMPEG_BIN` (path to ffmpeg). Default: `ffmpeg` in PATH.

---

## 2) FFmpeg 8.0+ supply strategy (no Docker)

App must support **either**:
1) **Bundled static FFmpeg binary** shipped with the app, *or*
2) **Local build/install** performed by the user/admin.

### If bundling:
- ship `ffmpeg` (+ `ffprobe`) in `./bin/`.
- app auto‑selects `./bin/ffmpeg` unless user overrides `FFMPEG_BIN`.

### If building from source:
- document build flags so AV1 encoders exist:
  - `--enable-libsvtav1`
  - `--enable-libaom`
  - `--enable-librav1e` (optional)
FFmpeg needs these libraries present at configure time. citeturn0search25turn0search5turn0search0

---

## 3) Software encoder selection policy

### Default encoder order
1) **SVT‑AV1‑PSY** (if `libsvtav1` links against PSY build)
2) **Mainline SVT‑AV1** (`libsvtav1`)
3) **libaom‑av1** (fallback)
4) **rav1e** (fallback)

SVT‑AV1 scales well on CPU and is the default production encoder. citeturn1search4turn0search0  
SVT‑AV1‑PSY adds perceptual/grain‑friendly tuning. citeturn1search1turn1search7

### Encoder detection
At runtime, app must check:
- `ffmpeg -hide_banner -encoders | grep -E "libsvtav1|libaom-av1|librav1e"`
- Choose best available per order above.

---

## 4) Quality‑max encoding rules (shared logic)

### 4.1 Always classify source first
Classify into buckets:

A) **REMUX / DISC MASTER**  
Blu‑ray/UHD remux, ProRes/mezzanine, high‑bitrate masters.  
Goal: preserve grain, micro‑texture, gradients.

B) **WEB‑DL / STREAMING DOWNLOAD**  
Already delivery‑encoded.  
Goal: avoid compounding artifacts; re‑encode only if asked.

C) **LOW‑QUALITY RIP**  
Already artifacted/low bitrate.  
Goal: size reduction OK.

If uncertain, assume the higher‑quality bucket.

### 4.2 Mandatory test clip for bucket A
Before full encode:
- generate 30–60s test covering:
  - darkest scene
  - most grain/texture
  - fastest motion
- user reviews
- if artifacts appear → lower CRF by 2 or slow preset.

This is required for remuxes because grain and shadows are hardest to preserve. citeturn0search6turn1search5

### 4.3 Bit depth / pixel format
- If source is 10‑bit or HDR: **keep 10‑bit output**.
- Use `yuv420p10le` / `p010le` pipeline, not 8‑bit truncation.
SVT‑AV1 supports 8‑bit & 10‑bit 4:2:0. citeturn1search4

---

## 5) SVT‑AV1 quality‑max profiles (CPU)

Use constant‑quality **CRF** mode. Lower CRF = higher quality.

### Bucket A: REMUX / DISC MASTER
**Goal:** near‑transparent archival.

- **1080p remux:** start `CRF 18` (allowed `16–21`)
- **2160p/UHD remux:** start `CRF 20` (allowed `18–22`)
- **Preset:** `-preset 2–4` (default `3`, go `2` for very grainy content) citeturn0search13turn0search4
- **Tunes / grain:**
  - If PSY build available, default to perceptual tune (PSY defaults / Tune 2). citeturn1search7
  - For visible grain, enable film‑grain synthesis:
    - `-svtav1-params film-grain=6–10` (start `8`) citeturn0search6turn1search6
  - If using PSY/Mod with Tune 3 for grain:
    - set `-svtav1-params tune=3` (optimized for grain retention). citeturn1search5turn1search7

### Bucket B: WEB‑DL
**Goal:** conservative re‑encode.

- If source codec already **HEVC/AV1/VP9 and visually clean**, default to **no re‑encode** unless user requests.  
- If re‑encoding H.264 WEB‑DL:
  - **1080p:** start `CRF 26` (allowed `24–29`)
  - **2160p:** start `CRF 28` (allowed `26–32`)
  - **Preset:** `4–6` (default `5`)

### Bucket C: LOW‑QUALITY RIP
- start `CRF 30` (allowed `30–35`)
- preset `6–8`
- no film‑grain synthesis.

---

## 6) Command templates the app should generate

### 6.1 REMUX test clip (SVT‑AV1)
```bash
{FFMPEG_BIN} -ss {START} -t {DUR} -i "{IN}" \
  -map 0:v:0 -map 0:a? -map 0:s? \
  -vf "{FILTERS},format=yuv420p10le" \
  -c:v libsvtav1 -crf 18 -preset 3 \
  -pix_fmt yuv420p10le \
  -svtav1-params tune=3:film-grain=8 \
  -c:a copy -c:s copy "{OUT_TEST}.mkv"
```

### 6.2 Full remux encode (after approval)
Same flags, no `-ss/-t`, output `{OUT_FINAL}.mkv`.

### 6.3 WEB‑DL encode
```bash
{FFMPEG_BIN} -i "{IN}" \
  -vf "{FILTERS},format=yuv420p10le" \
  -c:v libsvtav1 -crf 26 -preset 5 \
  -pix_fmt yuv420p10le \
  -c:a copy -c:s copy "{OUT}.mkv"
```

---

## 7) Acceptance criteria
- App runs encodes **without Docker installed**.
- App refuses to start if FFmpeg < 8.0.
- App detects AV1 software encoders and prefers SVT‑AV1‑PSY/SVT‑AV1.
- REMUX bucket forces test‑clip workflow.
- Default outputs prioritize quality, even if files are large.
