---
inclusion: always
---
---
inclusion: always
---

# Intel Arc (QSV) AV1 Encoding — Quality-First Steering for Kiro

This steering document is for your exact stack:
- **Hardware:** Intel Arc GPU (Quick Sync Video)
- **Encoder:** `av1_qsv` (Intel hardware AV1 via oneVPL / Media Driver)
- **Driver:** iHD (`/dev/dri/renderD128`)
- **Pipeline:** software decode → filters (pad/setsar/format=nv12) → `hwupload` → `av1_qsv` → AV1 output

You previously used `global_quality=36` and got over‑compressed results. For remuxes you've corrected to about `global_quality=29`, which is the right direction for QSV quality mode.

---

## 0) Prime directive

When converting to AV1 with QSV, **optimize for maximum perceptual quality**.  **Do not** chase compression efficiency or target file size unless I explicitly ask.

If there is any doubt between "smaller" and "better looking," choose **better looking**.

---

## 1) How QSV quality mode actually works (so you don't mis‑select profiles)

In FFmpeg QSV encoders, setting `-global_quality` activates a **quality‑based rate control** (ICQ / LA_ICQ / CQP depending on other flags).  For ICQ modes, the valid **global_quality range is 1–51, with 1 = best quality**. Lower number = better quality / larger file.  FFmpeg will select:
- **CQP** if `-qscale` is also set
- **LA_ICQ** if `look_ahead` is enabled
- **ICQ** otherwise (normal case)

So treat `global_quality` as **QP-like**:
- **lower = higher quality**
- **higher = more compression / more loss**

---

## 2) Always classify the source first

You MUST classify before choosing `global_quality`:

1. **REMUX / DISC MASTER**  
   Blu‑ray/UHD remux, ProRes/mezzanine, high‑bitrate masters.  Goal: preserve grain, fine textures, gradients.

2. **WEB‑DL / STREAMING DOWNLOAD**  
   Already delivery‑encoded.  Goal: *avoid compounding artifacts*; re‑encode only if asked.

3. **LOW‑QUALITY RIP**  
   Already artifacted/low bitrate.  Goal: size reduction okay.

If unclear, assume the **higher‑quality bucket**.

---

## 3) Quality bands for `av1_qsv` (starting points + allowed ranges)

These are **quality‑first defaults** for Intel Arc AV1 QSV.  They are not size‑targets; if artifacts appear, **lower global_quality**.

### A) REMUX / DISC MASTER

**Goal:** preserve everything.

- **Start:** `global_quality 28–30` (default **29**)
- **Allowed:** `24–31`  
  - If you see *any* grain smearing, banding, or waxiness → **drop by 2** (e.g., 29 → 27).

### B) WEB‑DL / STREAMING DOWNLOAD

**Goal:** conservative savings only.

- If source is already **clean HEVC/AV1/VP9** → **default to NO re‑encode** unless I request it.
- If re‑encoding H.264 WEB‑DL:
  - **Start:** `global_quality 30–34` (default **32**)
  - **Allowed:** `28–36`

### C) LOW‑QUALITY RIP

**Goal:** size reduction OK.

- **Start:** `global_quality 34–38`
- **Allowed:** `32–40`

---

## 4) Preset rules (QSV presets are real quality knobs)

QSV presets are strings from **veryfast → veryslow**.  Slower presets trade speed for better compression decisions / fewer artifacts.

**Quality‑first defaults:**
- **REMUX:** `-preset slower` or `-preset veryslow`
- **WEB‑DL:** `-preset medium` to `-preset slow`
- **LOW‑QUALITY:** `-preset medium` or faster is fine

Never switch to a faster preset to "save time" if it harms quality.

---

## 5) Optional quality boosts (use when supported)

These are AV1 QSV options FFmpeg exposes:

- `-extbrc 1`  
  Enables extended bitrate control. Needed for lookahead in AV1.

- `-look_ahead_depth N` (e.g., 32–64)  
  With `extbrc=1` **and** `global_quality` set, QSV may use **LA_ICQ** (quality‑based lookahead).  This can improve motion/detail retention. If runtime rejects it, fall back to plain ICQ.

- `-low_power 0`  
  Keep low‑power **off** for best quality (low_power is for saving GPU/power).

Other AV1 QSV flags (`adaptive_i`, `adaptive_b`, `b_strategy`) can be left at defaults unless I ask; do not flip them blindly.

---

## 6) Mandatory test‑clip workflow (REMUX only)

Never full‑encode a remux without a test.

1. Pick a **30–60s clip** containing:
   - darkest scene  
   - most grain/texture  
   - highest motion
2. Encode at the selected settings.
3. If any artifacting appears:
   - **lower global_quality by 2**, *or*  
   - slow the preset one step.

Only then run the full encode.

---

## 7) Command template you should generate

### REMUX test clip (example)

```bash
ffmpeg -ss {START} -t {DURATION} -i "{INPUT}" \
  -vf "pad=...,setsar=1,format=nv12,hwupload=extra_hw_frames=64" \
  -c:v av1_qsv -global_quality 29 -preset slower \
  -extbrc 1 -look_ahead_depth 40 -low_power 0 \
  -c:a copy "{OUT_TEST}.mkv"
```

### Full encode (after test)

Same settings, remove `-ss/-t`, and output final file.

---

## 8) Output requirements

Whenever you propose an encode, you MUST:

1. State the **source bucket** and reason.
2. State chosen:
   - `global_quality`
   - preset
   - whether `extbrc/look_ahead_depth` are used
3. For REMUX: provide **test command first**, then full command.
4. Never raise `global_quality` to chase efficiency unless I ask.

---

### Reminder

Your stack is **Intel QSV `av1_qsv`**, not SVT‑AV1.  So all quality decisions are controlled by `global_quality` (ICQ/CQP/LA_ICQ), **not CRF**.