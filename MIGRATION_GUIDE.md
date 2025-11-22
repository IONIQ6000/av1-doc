# Migration Guide: Docker Hardware Encoding → Native Software AV1 Encoding

This comprehensive guide helps you migrate from Docker-based Intel QSV hardware encoding to native software AV1 encoding with a quality-first approach.

## Table of Contents

1. [Overview](#overview)
2. [What's Changing](#whats-changing)
3. [Prerequisites](#prerequisites)
4. [Step-by-Step Migration](#step-by-step-migration)
5. [Configuration Changes](#configuration-changes)
6. [Expected Behavior Changes](#expected-behavior-changes)
7. [Troubleshooting](#troubleshooting)
8. [Performance Tuning](#performance-tuning)
9. [Rollback Plan](#rollback-plan)
10. [FAQ](#faq)

## Overview

This migration transitions your AV1 encoding system from:
- **Docker-based** FFmpeg execution → **Native** FFmpeg execution
- **Intel QSV hardware** encoding (GPU) → **Software** encoding (CPU)
- **Speed-optimized** approach → **Quality-first** approach

### Why Migrate?

**Benefits**:
- ✅ **Superior quality**: Quality-first approach preserves grain, texture, and detail
- ✅ **No Docker dependency**: Simpler deployment and maintenance
- ✅ **Intelligent source handling**: Automatic classification (REMUX, WEB-DL, LOW-QUALITY)
- ✅ **Test clip workflow**: Validate quality before full encode (REMUX sources)
- ✅ **Perceptual tuning**: SVT-AV1-PSY support for grain-optimized encoding
- ✅ **Better bit depth handling**: Automatic 10-bit preservation and HDR metadata

**Trade-offs**:
- ⚠️ **Much slower**: 10-20x slower encoding (CPU vs GPU)
- ⚠️ **Higher CPU usage**: 100% CPU utilization during encoding
- ⚠️ **More power consumption**: CPU encoding uses more power than GPU
- ⚠️ **Larger files**: Quality-first approach produces larger files (especially REMUX)

## What's Changing

### Architecture Changes

| Component | Old (Docker/Hardware) | New (Native/Software) |
|-----------|----------------------|----------------------|
| **FFmpeg Execution** | Docker container | Direct subprocess |
| **Encoding Method** | Intel QSV (av1_qsv) | Software (libsvtav1/libaom/librav1e) |
| **Encoding Speed** | 60-120 fps (1080p) | 0.5-4 fps (1080p) |
| **Quality Approach** | Balanced | Quality-first |
| **Docker Required** | Yes | No |
| **GPU Usage** | High (80-100%) | None |
| **CPU Usage** | Low (10-30%) | High (100%) |


### Configuration Changes

| Setting | Old (Docker) | New (Native) | Status |
|---------|-------------|--------------|--------|
| `docker_image` | Required | N/A | ❌ **Remove** |
| `docker_bin` | Required | N/A | ❌ **Remove** |
| `gpu_device` | Required | N/A | ❌ **Remove** |
| `ffmpeg_bin` | N/A | Optional (default: "ffmpeg") | ✅ **Add** |
| `ffprobe_bin` | N/A | Optional (default: "ffprobe") | ✅ **Add** |
| `require_ffmpeg_version` | N/A | Optional (default: "8.0") | ✅ **Add** |
| `force_reencode` | N/A | Optional (default: false) | ✅ **Add** |
| `enable_test_clip_workflow` | N/A | Optional (default: true) | ✅ **Add** |
| `test_clip_duration` | N/A | Optional (default: 45) | ✅ **Add** |
| `preferred_encoder` | N/A | Optional (auto-detect) | ✅ **Add** |

### Quality Settings Changes

| Source Type | Old (QSV) | New (Software) | Quality Impact |
|-------------|-----------|----------------|----------------|
| **REMUX** | global_quality 29-36 | CRF 18-20, preset 3 | 🔼 Better grain/detail |
| **WEB-DL** | global_quality 32-36 | CRF 26-28, preset 5 | 🔼 Less artifact compounding |
| **LOW-QUALITY** | global_quality 36-40 | CRF 30, preset 6 | ≈ Similar |

## Prerequisites

Before starting migration, ensure you have:

1. **FFmpeg 8.0 or later** with AV1 encoder support
2. **At least one AV1 software encoder**: libsvtav1, libaom-av1, or librav1e
3. **Sufficient CPU resources**: 8+ cores recommended
4. **Adequate RAM**: 16GB+ for 4K content
5. **Backup of current configuration**: Always backup before migrating

### Quick Verification

```bash
# Check FFmpeg version (must be 8.0+)
ffmpeg -version | head -n1

# Check AV1 encoders (need at least one)
ffmpeg -encoders 2>/dev/null | grep -E "libsvtav1|libaom|librav1e"

# Expected output:
# ffmpeg version 8.0 Copyright (c) 2000-2024 the FFmpeg developers
# V..... libsvtav1            SVT-AV1(Scalable Video Technology for AV1) encoder (codec av1)
```

If FFmpeg 8.0+ is not installed, see [INSTALL_SOFTWARE_AV1.md](INSTALL_SOFTWARE_AV1.md) for installation instructions.


## Step-by-Step Migration

### Step 1: Backup Current System

**Critical**: Always backup before making changes.

```bash
# Backup configuration
sudo cp /etc/av1d/config.json /etc/av1d/config.json.backup

# Backup job state
sudo cp -r /var/lib/av1d/jobs /var/lib/av1d/jobs.backup

# Backup daemon binary (optional)
sudo cp /usr/local/bin/av1d /usr/local/bin/av1d.backup

# Stop daemon
sudo systemctl stop av1d
```

### Step 2: Install FFmpeg 8.0+ with AV1 Encoders

If you don't have FFmpeg 8.0+ installed, follow the [Software AV1 Installation Guide](INSTALL_SOFTWARE_AV1.md).

**Quick install options**:

**Ubuntu/Debian**:
```bash
sudo apt update && sudo apt install ffmpeg libsvtav1enc-dev
```

**Arch Linux**:
```bash
sudo pacman -S ffmpeg svt-av1
```

**macOS**:
```bash
brew install ffmpeg
```

**Verify installation**:
```bash
ffmpeg -version | head -n1
ffmpeg -encoders | grep libsvtav1
```

### Step 3: Update Configuration File

Edit `/etc/av1d/config.json` to remove Docker settings and add FFmpeg paths.

**Before** (Docker-based):
```json
{
  "library_roots": ["/media/movies", "/media/tv"],
  "min_bytes": 2147483648,
  "max_size_ratio": 0.90,
  "job_state_dir": "/var/lib/av1d/jobs",
  "scan_interval_secs": 60,
  "temp_output_dir": "/nvme/av1d-temp",
  "docker_image": "lscr.io/linuxserver/ffmpeg:version-8.0-cli",
  "docker_bin": "docker",
  "gpu_device": "/dev/dri"
}
```

**After** (Software-based):
```json
{
  "library_roots": ["/media/movies", "/media/tv"],
  "min_bytes": 2147483648,
  "max_size_ratio": 0.90,
  "job_state_dir": "/var/lib/av1d/jobs",
  "scan_interval_secs": 60,
  "temp_output_dir": "/nvme/av1d-temp",
  "ffmpeg_bin": "ffmpeg",
  "ffprobe_bin": "ffprobe"
}
```

**Required changes**:
- ❌ **Remove**: `docker_image`
- ❌ **Remove**: `docker_bin`
- ❌ **Remove**: `gpu_device`
- ✅ **Add**: `ffmpeg_bin` (default: `"ffmpeg"`)
- ✅ **Add**: `ffprobe_bin` (default: `"ffprobe"`)

**Optional new settings**:
```json
{
  "require_ffmpeg_version": "8.0",
  "force_reencode": false,
  "enable_test_clip_workflow": true,
  "test_clip_duration": 45,
  "preferred_encoder": "libsvtav1"
}
```


### Step 4: Rebuild and Reinstall Daemon

```bash
# Navigate to project directory
cd /path/to/av1-daemon

# Pull latest code (if using git)
git pull

# Rebuild project
cargo build --release

# Reinstall binaries
sudo install -m 755 target/release/av1d /usr/local/bin/av1d
sudo install -m 755 target/release/av1top /usr/local/bin/av1top

# Verify installation
av1d --version
```

### Step 5: Update Systemd Service

Edit `/etc/systemd/system/av1d.service` to remove Docker dependency and add resource limits.

**Before** (Docker-based):
```ini
[Unit]
Description=AV1 Transcoding Daemon
After=docker.service network.target
Requires=docker.service

[Service]
Type=simple
ExecStart=/usr/local/bin/av1d --config /etc/av1d/config.json
Restart=always
RestartSec=10
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
```

**After** (Software-based):
```ini
[Unit]
Description=AV1 Software Transcoding Daemon
After=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/av1d --config /etc/av1d/config.json
Restart=always
RestartSec=10
StandardOutput=journal
StandardError=journal

# Resource limits (adjust based on your system)
CPUQuota=800%      # Limit to 8 cores (800% = 8 cores)
MemoryMax=16G      # Limit to 16GB RAM
Nice=-5            # Higher priority (optional)

[Install]
WantedBy=multi-user.target
```

**Changes**:
- ❌ **Remove**: `After=docker.service`
- ❌ **Remove**: `Requires=docker.service`
- ✅ **Add**: `CPUQuota` to limit CPU usage (optional but recommended)
- ✅ **Add**: `MemoryMax` to limit memory usage (optional but recommended)
- ✅ **Add**: `Nice` for process priority (optional)

Reload systemd:
```bash
sudo systemctl daemon-reload
```

### Step 6: Test Configuration

**Important**: Test before starting as a service.

```bash
# Run daemon in foreground
av1d --config /etc/av1d/config.json
```

**Expected output**:
```
[INFO] FFmpeg version: 8.0.x
[INFO] Available AV1 encoders: libsvtav1
[INFO] Selected AV1 encoder: libsvtav1
[INFO] Starting daemon with 2 library roots
[INFO] Scanning /media/movies...
[INFO] Scanning /media/tv...
```

Press Ctrl+C to stop.

**If you see errors**, check the [Troubleshooting](#troubleshooting) section before proceeding.


### Step 7: Start Daemon

```bash
# Start daemon service
sudo systemctl start av1d

# Check status
sudo systemctl status av1d

# View logs
sudo journalctl -u av1d -f
```

**Verify daemon is running**:
```bash
# Check process
ps aux | grep av1d

# Check logs for successful startup
sudo journalctl -u av1d -n 50
```

### Step 8: Monitor First Encode

Use the TUI to monitor the first encode:

```bash
av1top
```

**What to expect**:
- **Much slower encoding**: 0.5-4 fps (vs 60-120 fps with hardware)
- **High CPU usage**: Near 100% CPU utilization
- **Better quality**: Larger files with superior visual quality
- **Test clip workflow**: REMUX sources will pause for user approval
- **Detailed logging**: Quality decisions logged with reasoning

**Example log output**:
```
[INFO] Classified source as REMUX tier (bitrate: 25.5 Mbps, confidence: 0.95)
[INFO] Selected CRF 18 for 1080p REMUX (quality-first approach)
[INFO] Selected preset 3 (slower) for maximum quality
[INFO] Enabling film-grain synthesis (value: 8)
[INFO] Extracting test clip for quality validation...
[INFO] Test clip ready: /tmp/av1d-temp/test-clip-12345.mkv
[INFO] Waiting for user approval before full encode...
```

### Step 9: Review Test Clip (REMUX Sources Only)

For REMUX sources, the daemon will extract a test clip and wait for approval.

**Review the test clip**:
```bash
# Find test clip path in logs
sudo journalctl -u av1d | grep "Test clip ready"

# Play test clip
mpv /tmp/av1d-temp/test-clip-12345.mkv
```

**Check for**:
- Grain preservation (no smoothing or waxiness)
- Detail retention (sharp textures)
- No banding in gradients
- No blocking artifacts

**Approve or adjust**:
- If quality is good: Approve (daemon will proceed)
- If artifacts present: Request lower CRF or slower preset
- If unacceptable: Reject and adjust settings

### Step 10: Clean Up Docker (Optional)

Once you've verified software encoding works correctly:

```bash
# Remove Docker image (optional)
docker rmi lscr.io/linuxserver/ffmpeg:version-8.0-cli

# Uninstall Docker (optional, only if not used for other purposes)
sudo apt remove docker.io docker-ce docker-ce-cli containerd.io

# Remove Docker data (optional, be careful!)
sudo rm -rf /var/lib/docker
```

**Warning**: Only remove Docker if you're not using it for other applications.


## Configuration Changes

### Complete Configuration Comparison

**Old Configuration** (Docker/Hardware):
```json
{
  "library_roots": ["/media/movies", "/media/tv"],
  "min_bytes": 2147483648,
  "max_size_ratio": 0.90,
  "job_state_dir": "/var/lib/av1d/jobs",
  "scan_interval_secs": 60,
  "temp_output_dir": "/nvme/av1d-temp",
  "stuck_job_timeout_secs": 3600,
  "stuck_job_file_inactivity_secs": 600,
  "stuck_job_check_enable_process": true,
  "stuck_job_check_enable_file_activity": true,
  "docker_image": "lscr.io/linuxserver/ffmpeg:version-8.0-cli",
  "docker_bin": "docker",
  "gpu_device": "/dev/dri"
}
```

**New Configuration** (Native/Software):
```json
{
  "library_roots": ["/media/movies", "/media/tv"],
  "min_bytes": 2147483648,
  "max_size_ratio": 0.90,
  "job_state_dir": "/var/lib/av1d/jobs",
  "scan_interval_secs": 60,
  "temp_output_dir": "/nvme/av1d-temp",
  "stuck_job_timeout_secs": 3600,
  "stuck_job_file_inactivity_secs": 600,
  "stuck_job_check_enable_process": true,
  "stuck_job_check_enable_file_activity": true,
  "ffmpeg_bin": "ffmpeg",
  "ffprobe_bin": "ffprobe",
  "require_ffmpeg_version": "8.0",
  "force_reencode": false,
  "enable_test_clip_workflow": true,
  "test_clip_duration": 45,
  "preferred_encoder": "libsvtav1"
}
```

### New Configuration Options Explained

#### `ffmpeg_bin` (string, default: "ffmpeg")
Path to FFmpeg binary. Use default if FFmpeg is in PATH, or specify full path:
```json
"ffmpeg_bin": "/usr/local/bin/ffmpeg"
```

#### `ffprobe_bin` (string, default: "ffprobe")
Path to FFprobe binary. Use default if FFprobe is in PATH, or specify full path:
```json
"ffprobe_bin": "/usr/local/bin/ffprobe"
```

#### `require_ffmpeg_version` (string, default: "8.0")
Minimum FFmpeg version required. Daemon will fail to start if version is lower:
```json
"require_ffmpeg_version": "8.0"
```

#### `force_reencode` (boolean, default: false)
Force re-encoding even for clean WEB-DL sources with modern codecs (HEVC/AV1/VP9):
```json
"force_reencode": false  // Skip clean WEB-DL (recommended)
"force_reencode": true   // Always re-encode
```

#### `enable_test_clip_workflow` (boolean, default: true)
Enable test clip extraction and approval for REMUX sources:
```json
"enable_test_clip_workflow": true   // Validate quality (recommended)
"enable_test_clip_workflow": false  // Skip test clips
```

#### `test_clip_duration` (integer, default: 45)
Test clip duration in seconds (30-60 recommended):
```json
"test_clip_duration": 45
```

#### `preferred_encoder` (string, optional)
Override automatic encoder selection. Options: "libsvtav1", "libaom-av1", "librav1e":
```json
"preferred_encoder": "libsvtav1"  // Force SVT-AV1
```


## Expected Behavior Changes

### Encoding Speed

**Dramatic slowdown expected** - this is normal for software encoding.

| Content Type | Hardware (QSV) | Software (SVT-AV1) | Slowdown Factor |
|--------------|----------------|-------------------|-----------------|
| 1080p REMUX | 60-120 fps | 0.5-1 fps | 60-120x slower |
| 1080p WEB-DL | 80-150 fps | 2-4 fps | 20-40x slower |
| 4K REMUX | 20-40 fps | 0.2-0.5 fps | 40-100x slower |
| 4K WEB-DL | 30-60 fps | 0.8-2 fps | 15-40x slower |

**Real-world examples**:
- 2-hour 1080p REMUX: 2 minutes (hardware) → 2-4 hours (software)
- 2-hour 4K REMUX: 6 minutes (hardware) → 4-10 hours (software)
- 45-minute TV episode: 30 seconds (hardware) → 30-60 minutes (software)

### Quality Improvements

#### REMUX Sources (Blu-ray, High-Quality Masters)

**Old (Hardware)**:
- global_quality 29-36
- Fast encoding (60-120 fps)
- Some grain smoothing
- Occasional banding in gradients

**New (Software)**:
- CRF 18-20, preset 3 (slower)
- Slow encoding (0.5-1 fps)
- ✅ Excellent grain preservation
- ✅ No banding in gradients
- ✅ Sharper detail retention
- ✅ Film-grain synthesis enabled
- ✅ Test clip validation

#### WEB-DL Sources (Streaming Downloads)

**Old (Hardware)**:
- global_quality 32-36
- Always re-encoded
- Some artifact compounding

**New (Software)**:
- CRF 26-28, preset 5
- ✅ Skips clean HEVC/AV1/VP9 (unless forced)
- ✅ Conservative re-encoding for H.264
- ✅ No artifact compounding
- ✅ Appropriate quality selection

#### LOW-QUALITY Sources (Low-Bitrate Rips)

**Old (Hardware)**:
- global_quality 36-40
- Fast encoding

**New (Software)**:
- CRF 30, preset 6
- ≈ Similar quality
- ✅ Size-optimized encoding
- ✅ Fast presets for degraded content

### File Size Changes

**Hardware encoding** (QSV):
- Aggressive compression
- Smaller files (50-70% of original)
- Some quality loss

**Software encoding** (Quality-first):
- Quality-prioritized compression
- **REMUX**: Larger files (60-90% of original) - **This is expected**
- **WEB-DL**: Similar or smaller (many skipped)
- **LOW-QUALITY**: Smaller files (40-60% of original)

**Example file sizes**:
```
Original REMUX: 50 GB
Hardware encode: 30 GB (60% of original)
Software encode: 40 GB (80% of original) - Better quality!

Original WEB-DL: 10 GB (HEVC)
Hardware encode: 7 GB (70% of original)
Software encode: Skipped (already modern codec)

Original LOW-QUALITY: 2 GB
Hardware encode: 1.2 GB (60% of original)
Software encode: 1.0 GB (50% of original)
```


### CPU and Power Usage

**Hardware encoding** (QSV):
- Low CPU usage (10-30%)
- High GPU usage (80-100%)
- Lower power consumption (~50-100W)
- Cool and quiet

**Software encoding** (CPU):
- High CPU usage (100%)
- No GPU usage (0%)
- Higher power consumption (~150-300W)
- Hot and loud (fans running)

**Power consumption example** (24/7 encoding):
- Hardware: ~50W average = ~1.2 kWh/day = ~$0.15/day (at $0.12/kWh)
- Software: ~200W average = ~4.8 kWh/day = ~$0.58/day (at $0.12/kWh)

### New Workflow: Test Clip Approval (REMUX Only)

**New behavior**: REMUX sources require test clip approval before full encode.

**Workflow**:
1. Daemon detects REMUX source
2. Extracts 30-60 second test clip
3. Encodes test clip with proposed settings
4. **Pauses and waits for user approval**
5. User reviews test clip quality
6. User approves, adjusts, or rejects
7. Daemon proceeds with full encode (if approved)

**How to handle**:
```bash
# Monitor logs for test clip notification
sudo journalctl -u av1d -f

# Look for:
# [INFO] Test clip ready: /tmp/av1d-temp/test-clip-12345.mkv
# [INFO] Waiting for user approval...

# Review test clip
mpv /tmp/av1d-temp/test-clip-12345.mkv

# Approve via TUI or command file
# (Implementation-specific)
```

**To disable test clips** (not recommended):
```json
{
  "enable_test_clip_workflow": false
}
```

### Source Classification

**New feature**: Automatic source classification determines encoding parameters.

**Classification tiers**:

1. **REMUX** (Blu-ray, High-Quality Masters)
   - Bitrate > 15 Mbps (1080p) or > 40 Mbps (2160p)
   - Lossless audio codecs (TrueHD, DTS-HD MA, FLAC)
   - High bits-per-pixel ratio
   - **Encoding**: CRF 18-20, preset 3, film-grain enabled

2. **WEB-DL** (Streaming Downloads)
   - Modern codecs (HEVC, AV1, VP9)
   - Filename markers (WEB-DL, WEBRIP, NF, AMZN)
   - Clean visual quality
   - **Encoding**: CRF 26-28, preset 5, or skipped

3. **LOW-QUALITY** (Low-Bitrate Rips)
   - Bitrate < 5 Mbps (1080p)
   - Visible compression artifacts
   - Low bits-per-pixel ratio
   - **Encoding**: CRF 30, preset 6, fast

**Classification logging**:
```
[INFO] Classified source as REMUX tier
[INFO] Reason: High bitrate (25.5 Mbps), lossless audio (TrueHD)
[INFO] Confidence: 0.95
```


## Troubleshooting

### Daemon Won't Start

#### Error: "FFmpeg 8.0 or later required, found: 7.x"

**Cause**: FFmpeg version is too old.

**Solution**:
```bash
# Check FFmpeg version
ffmpeg -version

# Install FFmpeg 8.0+ (see INSTALL_SOFTWARE_AV1.md)
# Ubuntu/Debian:
sudo apt update && sudo apt install ffmpeg

# Arch Linux:
sudo pacman -S ffmpeg

# macOS:
brew install ffmpeg

# Or build from source (see INSTALL_SOFTWARE_AV1.md)
```

#### Error: "No AV1 software encoders detected"

**Cause**: FFmpeg was built without AV1 encoder support.

**Solution**:
```bash
# Check available encoders
ffmpeg -encoders | grep av1

# If no AV1 encoders listed, install encoder libraries
# Ubuntu/Debian:
sudo apt install libsvtav1enc-dev

# Arch Linux:
sudo pacman -S svt-av1

# Or rebuild FFmpeg with encoder support (see INSTALL_SOFTWARE_AV1.md)
```

#### Error: "FFmpeg binary not found at path: ffmpeg"

**Cause**: FFmpeg is not in PATH or specified path is incorrect.

**Solution**:
```bash
# Find FFmpeg location
which ffmpeg

# If not found, install FFmpeg
# If found, update config with full path:
{
  "ffmpeg_bin": "/usr/local/bin/ffmpeg",
  "ffprobe_bin": "/usr/local/bin/ffprobe"
}
```

#### Error: "Permission denied" when accessing library roots

**Cause**: Daemon user doesn't have read/write access to media directories.

**Solution**:
```bash
# Check permissions
ls -la /media/movies

# Fix permissions (adjust user/group as needed)
sudo chown -R av1d:av1d /media/movies

# Or run daemon as appropriate user
sudo systemctl edit av1d
# Add: User=your-user
```

### Encoding Issues

#### Encoding is Too Slow

**Expected behavior**: Software encoding is 10-20x slower than hardware.

**This is normal**. Software AV1 encoding prioritizes quality over speed.

**Options**:
1. **Accept slower speed**: Quality-first approach is intentionally slow
2. **Add more CPU cores**: Encoding scales with CPU cores
3. **Process fewer files**: Reduce library scan frequency
4. **Disable test clips**: Skip test clip workflow (not recommended)
5. **Use faster presets**: Modify quality calculator (reduces quality)

**Check encoding speed**:
```bash
# Monitor CPU usage
htop

# Check daemon logs for fps
sudo journalctl -u av1d -f | grep fps

# Expected: 0.5-4 fps for 1080p content
```

#### Out of Memory Errors

**Cause**: Insufficient RAM for encoding (especially 4K content).

**Solution**:
```bash
# Check available memory
free -h

# Increase system RAM (16GB+ recommended for 4K)

# Or limit daemon memory in systemd service:
sudo systemctl edit av1d
# Add:
[Service]
MemoryMax=16G

# Reload and restart
sudo systemctl daemon-reload
sudo systemctl restart av1d
```


#### Encoding Fails with "Encoder not found"

**Cause**: Selected encoder is not available.

**Solution**:
```bash
# Check available encoders
ffmpeg -encoders | grep av1

# Install missing encoder
# For SVT-AV1:
sudo apt install libsvtav1enc-dev  # Ubuntu/Debian
sudo pacman -S svt-av1              # Arch Linux

# Or remove preferred_encoder from config to use auto-detection
```

#### Test Clip Workflow Hangs

**Behavior**: Daemon pauses waiting for user input on REMUX sources.

**Expected**: REMUX sources require test clip approval before full encode.

**Solution**:
```bash
# Check daemon logs for test clip path
sudo journalctl -u av1d -f | grep "Test clip"

# Review test clip quality
mpv /tmp/av1d-temp/test-clip-12345.mkv

# Approve or reject via daemon interface
# (Implementation-specific, check daemon documentation)

# Or disable test clips (not recommended):
{
  "enable_test_clip_workflow": false
}
```

### Quality Issues

#### Output Quality is Poor

**Cause**: Incorrect source classification or settings.

**Solution**:
```bash
# Check classification in logs
sudo journalctl -u av1d | grep "Classified source"

# If misclassified, check source properties:
ffprobe -v quiet -print_format json -show_format -show_streams input.mkv

# Adjust classification thresholds in code if needed
# Or manually override settings
```

#### Files are Too Large

**Expected behavior**: Quality-first approach produces larger files (especially REMUX).

**This is intentional**. Quality is prioritized over file size.

**Options**:
1. **Accept larger files**: Quality-first approach preserves detail
2. **Adjust max_size_ratio**: Increase to allow larger outputs
3. **Modify CRF values**: Increase CRF for smaller files (reduces quality)

**Not recommended**: Sacrificing quality defeats the purpose of migration.

### Performance Issues

#### High CPU Usage

**Expected behavior**: Software encoding uses 100% CPU.

**This is normal**. CPU encoding is computationally intensive.

**Options**:
```bash
# Limit CPU usage in systemd service
sudo systemctl edit av1d
# Add:
[Service]
CPUQuota=400%  # Limit to 4 cores (400% = 4 cores)

# Reload and restart
sudo systemctl daemon-reload
sudo systemctl restart av1d
```

#### System Becomes Unresponsive

**Cause**: Encoding consuming all CPU resources.

**Solution**:
```bash
# Lower process priority
sudo systemctl edit av1d
# Add:
[Service]
Nice=10  # Lower priority (higher nice value)

# Or limit CPU quota (see above)
```


## Performance Tuning

### CPU Resource Management

#### Limit CPU Cores

Prevent daemon from using all CPU cores:

```bash
# Edit systemd service
sudo systemctl edit av1d

# Add CPU limit
[Service]
CPUQuota=400%  # Limit to 4 cores (400% = 4 cores)
CPUQuota=800%  # Limit to 8 cores (800% = 8 cores)

# Reload and restart
sudo systemctl daemon-reload
sudo systemctl restart av1d
```

#### Adjust Process Priority

Give daemon higher or lower priority:

```bash
# Edit systemd service
sudo systemctl edit av1d

# Higher priority (faster encoding, may impact system responsiveness)
[Service]
Nice=-10  # Higher priority (requires root)

# Lower priority (slower encoding, system remains responsive)
[Service]
Nice=10   # Lower priority

# Reload and restart
sudo systemctl daemon-reload
sudo systemctl restart av1d
```

#### CPU Affinity

Pin daemon to specific CPU cores:

```bash
# Edit systemd service
sudo systemctl edit av1d

# Pin to cores 0-7
[Service]
CPUAffinity=0-7

# Pin to specific cores (e.g., 8-15 for second CPU)
[Service]
CPUAffinity=8-15

# Reload and restart
sudo systemctl daemon-reload
sudo systemctl restart av1d
```

### Memory Management

#### Limit Memory Usage

Prevent daemon from consuming all RAM:

```bash
# Edit systemd service
sudo systemctl edit av1d

# Limit to 16GB
[Service]
MemoryMax=16G

# Limit to 32GB
[Service]
MemoryMax=32G

# Reload and restart
sudo systemctl daemon-reload
sudo systemctl restart av1d
```

### Parallel Encoding

Run multiple daemon instances for parallel encoding:

```bash
# Create separate configs for different library roots
sudo cp /etc/av1d/config.json /etc/av1d/config-movies.json
sudo cp /etc/av1d/config.json /etc/av1d/config-tv.json

# Edit each config to process different directories
# config-movies.json:
{
  "library_roots": ["/media/movies"],
  "job_state_dir": "/var/lib/av1d/jobs-movies"
}

# config-tv.json:
{
  "library_roots": ["/media/tv"],
  "job_state_dir": "/var/lib/av1d/jobs-tv"
}

# Create separate systemd services
sudo cp /etc/systemd/system/av1d.service /etc/systemd/system/av1d-movies.service
sudo cp /etc/systemd/system/av1d.service /etc/systemd/system/av1d-tv.service

# Edit ExecStart to use different configs
# av1d-movies.service:
ExecStart=/usr/local/bin/av1d --config /etc/av1d/config-movies.json

# av1d-tv.service:
ExecStart=/usr/local/bin/av1d --config /etc/av1d/config-tv.json

# Reload and start
sudo systemctl daemon-reload
sudo systemctl start av1d-movies av1d-tv
```

**Warning**: Parallel encoding will use more CPU and memory. Ensure your system has sufficient resources.

### Storage Optimization

#### Use Fast Temp Storage

Use NVMe or SSD for temporary output files:

```json
{
  "temp_output_dir": "/nvme/av1d-temp"
}
```

**Benefits**:
- Faster I/O during encoding
- Reduced wear on HDD media library
- Better performance for 4K content

#### Separate Temp and Output Storage

Keep temp files on fast storage, final output on media library:

```json
{
  "temp_output_dir": "/nvme/av1d-temp",
  "library_roots": ["/media/movies"]
}
```

Daemon will encode to temp storage, then move to final location.


## Rollback Plan

If you encounter issues and need to revert to Docker-based hardware encoding:

### Quick Rollback

```bash
# Stop new daemon
sudo systemctl stop av1d

# Restore backup configuration
sudo cp /etc/av1d/config.json.backup /etc/av1d/config.json

# Restore old daemon binary (if you kept it)
sudo cp /usr/local/bin/av1d.backup /usr/local/bin/av1d

# Restore systemd service
sudo systemctl daemon-reload

# Start daemon
sudo systemctl start av1d

# Verify
sudo systemctl status av1d
```

### Full Rollback

If you need to completely revert:

```bash
# Stop daemon
sudo systemctl stop av1d

# Restore configuration
sudo cp /etc/av1d/config.json.backup /etc/av1d/config.json

# Restore job state (if needed)
sudo rm -rf /var/lib/av1d/jobs
sudo cp -r /var/lib/av1d/jobs.backup /var/lib/av1d/jobs

# Reinstall Docker (if removed)
sudo apt install docker.io

# Pull Docker image
docker pull lscr.io/linuxserver/ffmpeg:version-8.0-cli

# Restore old daemon binary
# Option 1: From backup
sudo cp /usr/local/bin/av1d.backup /usr/local/bin/av1d

# Option 2: Rebuild from old commit
cd /path/to/av1-daemon
git checkout <old-commit>
cargo build --release
sudo install -m 755 target/release/av1d /usr/local/bin/av1d

# Restore systemd service
sudo cp /etc/systemd/system/av1d.service.backup /etc/systemd/system/av1d.service
sudo systemctl daemon-reload

# Start daemon
sudo systemctl start av1d

# Verify
sudo systemctl status av1d
sudo journalctl -u av1d -f
```

### Hybrid Approach

Run both hardware and software encoding simultaneously:

```bash
# Keep old daemon as av1d-qsv
sudo cp /usr/local/bin/av1d.backup /usr/local/bin/av1d-qsv

# Install new daemon as av1d-software
sudo install -m 755 target/release/av1d /usr/local/bin/av1d-software

# Create separate configs
sudo cp /etc/av1d/config.json.backup /etc/av1d/config-qsv.json
sudo cp /etc/av1d/config.json /etc/av1d/config-software.json

# Edit configs to process different directories
# config-qsv.json (hardware):
{
  "library_roots": ["/media/tv"],
  "docker_image": "lscr.io/linuxserver/ffmpeg:version-8.0-cli"
}

# config-software.json (software):
{
  "library_roots": ["/media/movies"],
  "ffmpeg_bin": "ffmpeg"
}

# Create separate systemd services
sudo cp /etc/systemd/system/av1d.service.backup /etc/systemd/system/av1d-qsv.service
sudo cp /etc/systemd/system/av1d.service /etc/systemd/system/av1d-software.service

# Edit ExecStart
# av1d-qsv.service:
ExecStart=/usr/local/bin/av1d-qsv --config /etc/av1d/config-qsv.json

# av1d-software.service:
ExecStart=/usr/local/bin/av1d-software --config /etc/av1d/config-software.json

# Start both
sudo systemctl daemon-reload
sudo systemctl start av1d-qsv av1d-software
```

This allows you to:
- Use hardware encoding for TV shows (speed)
- Use software encoding for movies (quality)
- Compare results before fully committing


## FAQ

### General Questions

#### Q: Should I migrate to software encoding?

**A**: Depends on your priorities:

**Migrate if**:
- Quality is paramount
- You have CPU resources (8+ cores)
- You want to eliminate Docker dependency
- You're encoding REMUX/high-quality sources
- You can accept 10-20x slower encoding

**Stay on hardware if**:
- Speed is critical
- You have limited CPU resources
- You need high throughput
- You're encoding large volumes of content
- Power consumption is a concern

#### Q: Can I test software encoding before fully migrating?

**A**: Yes! Run software encoding on a test directory:

```bash
# Create test config
cat > /tmp/test-config.json <<EOF
{
  "library_roots": ["/tmp/test-media"],
  "job_state_dir": "/tmp/test-jobs",
  "temp_output_dir": "/tmp/test-temp",
  "ffmpeg_bin": "ffmpeg",
  "ffprobe_bin": "ffprobe"
}
EOF

# Create test directory
mkdir -p /tmp/test-media /tmp/test-jobs /tmp/test-temp

# Copy a test file
cp /media/movies/test-movie.mkv /tmp/test-media/

# Run daemon with test config
av1d --config /tmp/test-config.json
```

Compare results before migrating your full library.

#### Q: Will my existing job state work?

**A**: Yes, job state files are compatible. Pending jobs will be processed with the new software encoding. Completed jobs remain unchanged.

#### Q: Can I use both hardware and software encoding?

**A**: Yes, run two daemon instances with different configurations (see [Hybrid Approach](#hybrid-approach) in Rollback Plan).

### Performance Questions

#### Q: Can I speed up software encoding?

**A**: Yes, but at the cost of quality:

1. **Use faster presets**: Modify quality calculator (not recommended for REMUX)
2. **Increase CRF values**: Lower quality, smaller files
3. **Skip test clip workflow**: Disable `enable_test_clip_workflow`
4. **Add more CPU cores**: Encoding scales with cores
5. **Use libaom-av1**: Slower but higher quality (opposite of speeding up)

**Not recommended**: The quality-first approach intentionally prioritizes quality over speed.

#### Q: How much slower is software encoding?

**A**: 10-20x slower than hardware encoding:
- Hardware: 60-120 fps (1080p REMUX)
- Software: 0.5-1 fps (1080p REMUX)

A 2-hour movie that took 2 minutes with hardware will take 2-4 hours with software.

#### Q: Why is encoding so slow?

**A**: Software AV1 encoding is computationally intensive:
- AV1 is a complex codec (more efficient than H.264/HEVC)
- Quality-first presets (slower = better quality)
- CPU encoding (no hardware acceleration)
- Film-grain synthesis (additional processing)

This is expected and intentional for maximum quality.

### Quality Questions

#### Q: Will quality actually be better?

**A**: Yes, especially for REMUX sources:
- Better grain preservation (no smoothing)
- No banding in gradients
- Sharper detail retention
- Film-grain synthesis
- Lower CRF values (higher quality)
- Slower presets (better compression decisions)

Compare test clips to see the difference.

#### Q: Why are files larger?

**A**: Quality-first approach prioritizes quality over file size:
- Lower CRF values = higher quality = larger files
- Slower presets = better quality = larger files
- Film-grain preservation = larger files
- No aggressive compression

**This is intentional**. If you want smaller files, you're sacrificing quality.

#### Q: What about 10-bit and HDR content?

**A**: Fully supported:
- Automatic 10-bit detection and preservation
- HDR metadata preservation (PQ, HLG, bt2020)
- Correct pixel format selection (yuv420p10le)
- No upconversion of 8-bit content

Software encoding handles bit depth better than hardware.


### Configuration Questions

#### Q: Do I need to specify FFmpeg paths?

**A**: Only if FFmpeg is not in your PATH:

```bash
# Check if FFmpeg is in PATH
which ffmpeg

# If found, use defaults:
{
  "ffmpeg_bin": "ffmpeg"
}

# If not found or using custom location:
{
  "ffmpeg_bin": "/usr/local/bin/ffmpeg",
  "ffprobe_bin": "/usr/local/bin/ffprobe"
}
```

#### Q: Which AV1 encoder should I use?

**A**: Automatic detection selects the best available:

**Priority order**:
1. **SVT-AV1-PSY** (best quality, perceptual tuning)
2. **libsvtav1** (best speed/quality balance)
3. **libaom-av1** (highest quality, slowest)
4. **librav1e** (good quality, moderate speed)

**Recommendation**: Use libsvtav1 (most widely available, good balance).

To override:
```json
{
  "preferred_encoder": "libsvtav1"
}
```

#### Q: Should I enable test clip workflow?

**A**: Yes, for REMUX sources (recommended):

```json
{
  "enable_test_clip_workflow": true
}
```

**Benefits**:
- Validate quality before full encode
- Catch artifacts early
- Adjust settings if needed
- Avoid wasting hours on bad encodes

**Disable only if**:
- You trust the automatic settings
- You're encoding WEB-DL/LOW-QUALITY (test clips are skipped anyway)
- You want fully automated workflow

#### Q: What is force_reencode?

**A**: Controls re-encoding of clean WEB-DL sources:

```json
{
  "force_reencode": false  // Skip clean HEVC/AV1/VP9 (recommended)
}
```

**Default (false)**: Skip re-encoding WEB-DL sources already encoded with modern codecs (HEVC, AV1, VP9). This avoids compounding artifacts and saves time.

**Set to true**: Always re-encode, even clean WEB-DL sources. Use if you want consistent AV1 encoding for all files.

### Troubleshooting Questions

#### Q: Daemon won't start, what should I check?

**A**: Check in this order:

1. **FFmpeg version**:
   ```bash
   ffmpeg -version | head -n1
   # Must be 8.0+
   ```

2. **AV1 encoders**:
   ```bash
   ffmpeg -encoders | grep av1
   # Must have at least one
   ```

3. **Configuration**:
   ```bash
   cat /etc/av1d/config.json
   # Check for syntax errors
   ```

4. **Permissions**:
   ```bash
   ls -la /media/movies
   # Check read/write access
   ```

5. **Logs**:
   ```bash
   sudo journalctl -u av1d -n 50
   # Check for error messages
   ```

#### Q: Encoding fails, how do I debug?

**A**: Check daemon logs:

```bash
# View recent logs
sudo journalctl -u av1d -n 100

# Follow logs in real-time
sudo journalctl -u av1d -f

# Look for error messages
sudo journalctl -u av1d | grep -i error
```

Common issues:
- FFmpeg command errors
- Insufficient disk space
- Permission denied
- Out of memory

#### Q: Test clip workflow is confusing, how does it work?

**A**: For REMUX sources only:

1. Daemon detects REMUX source
2. Extracts 30-60 second test clip
3. Encodes test clip with proposed settings
4. **Pauses and waits for approval**
5. User reviews test clip
6. User approves/adjusts/rejects
7. Daemon proceeds with full encode

**To review test clip**:
```bash
# Find test clip path in logs
sudo journalctl -u av1d | grep "Test clip ready"

# Play test clip
mpv /tmp/av1d-temp/test-clip-12345.mkv
```

**To disable** (not recommended):
```json
{
  "enable_test_clip_workflow": false
}
```


### Migration Questions

#### Q: How long does migration take?

**A**: 30-60 minutes for the migration process itself:
- 10 minutes: Install FFmpeg 8.0+
- 10 minutes: Update configuration
- 10 minutes: Rebuild and reinstall daemon
- 10 minutes: Test and verify
- 10 minutes: Monitor first encode

Actual encoding time depends on your library size.

#### Q: Will migration affect my existing encodes?

**A**: No, completed encodes are not affected. Only new encodes will use software encoding.

#### Q: Can I migrate gradually?

**A**: Yes, use the hybrid approach:
- Keep hardware encoding for TV shows
- Use software encoding for movies
- Compare results before fully committing

See [Hybrid Approach](#hybrid-approach) in Rollback Plan.

#### Q: What if I need to rollback?

**A**: Easy rollback with backups:
1. Stop daemon
2. Restore backup configuration
3. Restore old daemon binary
4. Restart daemon

See [Rollback Plan](#rollback-plan) for detailed instructions.

#### Q: Will Docker be completely removed?

**A**: Docker is no longer required, but you can keep it:
- Remove Docker if not used for other purposes
- Keep Docker if used for other applications
- Optional cleanup step in migration guide

### Cost Questions

#### Q: What about power consumption?

**A**: Software encoding uses more power:
- Hardware: ~50W average
- Software: ~200W average

**Cost example** (24/7 encoding at $0.12/kWh):
- Hardware: ~$0.15/day = ~$4.50/month
- Software: ~$0.58/day = ~$17.40/month

**Difference**: ~$13/month more for software encoding.

#### Q: Is the quality improvement worth the slowdown?

**A**: Depends on your use case:

**Worth it for**:
- Archival/preservation (REMUX sources)
- High-quality media library
- Blu-ray/UHD rips
- Content you'll watch repeatedly

**Not worth it for**:
- Temporary content
- Already-compressed WEB-DL
- High-volume processing
- Time-sensitive encoding

### Technical Questions

#### Q: What's the difference between CRF and global_quality?

**A**: Different rate control modes:

**Hardware (QSV)**:
- Uses `global_quality` (ICQ/LA_ICQ mode)
- Range: 1-51 (lower = better quality)
- Example: `global_quality 29`

**Software (SVT-AV1)**:
- Uses `CRF` (Constant Rate Factor)
- Range: 0-63 (lower = better quality)
- Example: `CRF 18`

Both control quality, but scales are different.

#### Q: What are presets?

**A**: Encoding speed/quality trade-offs:

**SVT-AV1 presets** (0-13):
- **0-2**: Extremely slow, maximum quality
- **3-4**: Slow, high quality (REMUX default)
- **5-6**: Medium, balanced (WEB-DL default)
- **7-9**: Fast, lower quality
- **10-13**: Very fast, lowest quality

**Slower presets** = better compression decisions = higher quality.

#### Q: What is film-grain synthesis?

**A**: Preserves natural film grain:
- Analyzes grain in source
- Encodes grain pattern separately
- Reconstructs grain during playback
- Prevents grain smoothing/loss

**Enabled for**: REMUX sources (value: 8)
**Disabled for**: WEB-DL, LOW-QUALITY sources


## Additional Resources

### Documentation

- **[Software AV1 Installation Guide](INSTALL_SOFTWARE_AV1.md)**: Complete installation instructions for FFmpeg 8.0+ with AV1 encoders
- **[FFmpeg Configuration Guide](FFMPEG_CONFIGURATION.md)**: Detailed FFmpeg binary path configuration
- **[Quick Start Guide](QUICK_START_SOFTWARE_AV1.md)**: Fast-track setup for software AV1 encoding
- **[Requirements Document](.kiro/specs/software-av1-encoding/requirements.md)**: Formal requirements specification
- **[Design Document](.kiro/specs/software-av1-encoding/design.md)**: Technical design and architecture

### External Resources

- **FFmpeg Documentation**: https://ffmpeg.org/documentation.html
- **SVT-AV1 GitHub**: https://gitlab.com/AOMediaCodec/SVT-AV1
- **SVT-AV1-PSY GitHub**: https://github.com/gianni-rosato/svt-av1-psy
- **libaom GitHub**: https://aomedia.googlesource.com/aom
- **librav1e GitHub**: https://github.com/xiph/rav1e
- **AV1 Codec Overview**: https://en.wikipedia.org/wiki/AV1

### Community and Support

- **Project Issues**: Report bugs and request features
- **Discussions**: Ask questions and share experiences
- **Wiki**: Community-contributed guides and tips

## Summary

### Migration Checklist

- [ ] Backup current configuration and job state
- [ ] Install FFmpeg 8.0+ with AV1 encoder support
- [ ] Verify FFmpeg version and encoder availability
- [ ] Update configuration file (remove Docker, add FFmpeg paths)
- [ ] Rebuild and reinstall daemon
- [ ] Update systemd service (remove Docker dependency)
- [ ] Test configuration in foreground
- [ ] Start daemon service
- [ ] Monitor first encode with TUI
- [ ] Review test clip quality (REMUX sources)
- [ ] Verify encoding quality and performance
- [ ] Clean up Docker (optional)

### Key Takeaways

**What You Gain**:
- ✅ Superior quality (grain preservation, detail retention)
- ✅ No Docker dependency (simpler deployment)
- ✅ Intelligent source handling (automatic classification)
- ✅ Test clip validation (quality assurance)
- ✅ Better bit depth handling (10-bit, HDR)

**What You Trade**:
- ⚠️ Much slower encoding (10-20x slower)
- ⚠️ Higher CPU usage (100% utilization)
- ⚠️ More power consumption (~4x more)
- ⚠️ Larger files (quality-first approach)

**Bottom Line**: If quality is your priority and you have CPU resources, software encoding is the way to go. If speed and throughput are critical, hardware encoding may be better for your use case.

### Next Steps

1. **Complete migration**: Follow the step-by-step guide
2. **Monitor first encode**: Use `av1top` to watch progress
3. **Review quality**: Compare outputs with original sources
4. **Adjust settings**: Fine-tune configuration if needed
5. **Provide feedback**: Report issues or share experiences

### Getting Help

If you encounter issues during migration:

1. **Check logs**: `sudo journalctl -u av1d -f`
2. **Verify FFmpeg**: `ffmpeg -version`
3. **Check encoders**: `ffmpeg -encoders | grep av1`
4. **Review configuration**: `cat /etc/av1d/config.json`
5. **Test manually**: `av1d --config /etc/av1d/config.json`
6. **Consult documentation**: See links above
7. **Ask for help**: Open an issue or discussion

---

**Good luck with your migration!** The quality improvements are worth the effort for high-quality sources.

