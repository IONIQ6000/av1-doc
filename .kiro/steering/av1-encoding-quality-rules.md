---
inclusion: always
---

# AV1 Encoding Rules (Quality-First, Remux vs WEB-DL)

You are helping me encode video to AV1 using **ffmpeg + SVT-AV1** (or SVT-AV1-PSY if available).  Your **primary goal is perceptual quality preservation**, *not* maximum compression ratio.

AV1 is typically more efficient than H.264/AVC by ~30–50% at equal quality, but that advantage **shrinks at high bitrates**, so you must not assume you can cut 90–95% bitrate on high-quality masters without visible loss.

---

## 1) Always classify the source BEFORE choosing CRF/preset

Classify into one of these three buckets. If unclear, assume the higher-quality bucket.

### A. REMUX / DISC MASTER

Examples: Blu-ray remux, UHD remux, ProRes/mezzanine, "full bitrate" releases.  These are usually near the top of the rate–distortion curve already; aggressive AV1 CRF will wipe grain/texture.

### B. WEB-DL / STREAMING DOWNLOAD

Examples: Netflix/Apple/Amazon WEB-DLs, already encoded delivery files.  Re-encoding can compound artifacts; aim for modest savings only.

### C. LOW-QUALITY RIP

Already artifacted or low-bitrate sources.  Quality ceiling is low; size reduction is acceptable.

---

## 2) Pick settings by bucket (starting points + allowed ranges)

CRF meaning: **lower CRF = higher quality / larger file.** SVT-AV1 CRF is 1–63.

Preset meaning: **lower preset number = better compression/quality but slower.**

### A. REMUX / DISC MASTER (quality-preserve)

**Goal:** keep grain, fine texture, subtle gradients.

**Defaults**
- Prefer **SVT-AV1-PSY** when possible for better perceptual/grain behavior.
- Preset: **2–4** (default **3**).
- Bit depth: keep **10-bit** if source is 10-bit/HDR.

**CRF targets**
- **1080p Blu-ray remux:** start **CRF 20–21**, allowed **18–23**.  (Community practice for Blu-ray masters commonly lands in ~16–23 depending on grain/detail.)
- **2160p UHD/HDR remux:** start **CRF 22–24**, allowed **20–26**.

**Grain handling**
- If visible grain/noise: enable **film grain synthesis**.  
  - Use `-svtav1-params film-grain=X` where X is **1–50**. Start around **6–10** (default **8**).
- Film grain synthesis can save large bitrate while keeping the *appearance* of grain.
- If using SVT-AV1-HDR/PSY, prefer **Tune 3 (Film Grain)** for grain retention and temporal consistency.

**Mandatory preflight**
- **Never full-encode a remux without a short test.**
  1. Pick **30–60 seconds** spanning:
     - darkest scene
     - most grain/texture-heavy scene
     - highest motion scene
  2. Encode test at the chosen settings.
  3. If grain smears/looks waxy → **lower CRF by 2** *or* slow preset by 1.

---

### B. WEB-DL / STREAMING DOWNLOAD (conservative)

**Goal:** modest size reduction without adding artifacts.

**Defaults**
- Preset: **4–6** (default **5**).
- If WEB-DL is already **HEVC/AV1/VP9 and looks clean**, default to **no re-encode** unless I explicitly request it.

**CRF targets**
- **1080p H.264 WEB-DL:** start **CRF 26–28**, allowed **24–30**.
- **2160p WEB-DL:** start **CRF 28–30**, allowed **26–32**.

**Grain**
- **Do not** enable grain synth unless:
  - source clearly has grain *and*
  - artifacts are low.

---

### C. LOW-QUALITY RIP (size-first OK)

**Goal:** compress more; quality already limited.

- Preset: **6–8**
- CRF: **30–35**
- No grain synthesis.

---

## 3) Output format you MUST follow

Whenever you propose an encode, you must:

1. **State the source bucket** and why.
2. List chosen:
   - CRF
   - preset
   - bit depth
   - tune (if any)
   - film-grain strength (if any)
3. If bucket = REMUX:
   - show **test-clip command first**
   - then the full encode command
4. Prefer **CRF/constant-quality** over ABR unless I give a target bitrate.

---

## 4) Reference SVT-AV1 parameter hints (for your commands)

- CRF example: `-crf 21` (lower = better).
- Preset example: `-preset 3` (lower = slower/better).
- Film grain synthesis: `-svtav1-params film-grain=8` (1–50).
- Tune 3 for grain (HDR/PSY builds): improves grain retention; recommended CRF window is broad but you will follow the bucket CRF targets above.

---

**Bottom line:**  High-quality sources get **low CRF + slower preset + grain protection**.  Already-compressed web sources get **higher CRF + moderate preset** and sometimes **no re-encode**.  You are optimizing for what looks best to humans, not what looks best on a bitrate chart.
