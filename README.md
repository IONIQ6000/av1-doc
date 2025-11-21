# AV1 Daemon (Rust)

A Rust-based AV1 transcoding daemon with ratatui TUI, using Docker ffmpeg with Intel QSV (Quick Sync Video) AV1 hardware acceleration.

## Overview

This project provides:
- **`av1d`**: A daemon that watches media libraries and transcodes files to AV1 using Intel QSV hardware acceleration
- **`av1top`**: A ratatui-based terminal UI for monitoring transcoding jobs

## Architecture

The project is organized as a Cargo workspace with three crates:

- **`crates/daemon`**: Core library with config, job management, scanning, ffprobe/ffmpeg wrappers, and classification
- **`crates/cli-daemon`**: Binary crate for the `av1d` daemon
- **`crates/cli-tui`**: Binary crate for the `av1top` monitoring TUI

## Requirements

- Rust toolchain (2021 edition)
- Docker installed and running
- **Intel Arc GPU** (A310, A380, or newer) with QSV support
- `/dev/dri` device available for GPU passthrough
- Intel media drivers with QSV/VPL support installed on host system

## Building

```bash
cargo build --release
```

Binaries will be in `target/release/`:
- `av1d` - the daemon
- `av1top` - the TUI

## Configuration

Create a JSON or TOML config file (optional, defaults are used if not provided):

```json
{
  "library_roots": ["/media/movies", "/media/tv"],
  "min_bytes": 2147483648,
  "max_size_ratio": 0.90,
  "job_state_dir": "/tmp/av1d-jobs",
  "scan_interval_secs": 60,
  "docker_image": "lscr.io/linuxserver/ffmpeg:version-8.0-cli",
  "docker_bin": "docker",
  "gpu_device": "/dev/dri"
}
```

## Usage

### Running the Daemon

```bash
./target/release/av1d --config /path/to/config.json
```

The daemon will:
1. Scan library roots for media files
2. Check for skip markers (`.av1skip` files)
3. Verify files are stable (not being copied)
4. Create jobs for candidates
5. Process jobs: probe, classify, transcode to AV1 using Intel QSV
6. Apply size gate (reject if new file > 90% of original)
7. Replace original with transcoded file (backing up as `.orig.mkv`)

### Running the TUI

```bash
./target/release/av1top
```

The TUI shows:
- CPU and memory usage
- Job table with status, file names, sizes, savings, duration, and reasons
- Status bar with job counts and controls

Press `q` to quit, `r` to refresh.

## Features

- **Docker-based ffmpeg**: Uses `lscr.io/linuxserver/ffmpeg:version-8.0-cli` image with FFmpeg 8.0
- **Intel QSV AV1**: Hardware-accelerated AV1 encoding via Intel Quick Sync Video
- **10-bit support**: Full support for both 8-bit and 10-bit AV1 encoding (HDR content preserved)
- **Smart classification**: Detects web-rip vs disc sources and applies appropriate flags
- **Russian track removal**: Automatically excludes Russian audio and subtitle tracks
- **Size gate**: Rejects transcodes that don't save enough space
- **Stable file detection**: Waits for files to finish copying before processing
- **Skip markers**: `.av1skip` files permanently mark files to skip
- **Explainable decisions**: `.why.txt` files explain why files were skipped

## Job State

Jobs are persisted as JSON files in the `job_state_dir`. Each job includes:
- Status (Pending, Running, Success, Failed, Skipped)
- Source and output paths
- File sizes (original and new)
- Timestamps
- Classification results
- Reasons for skip/failure

## Sidecar Files

- **`.av1skip`**: Permanently marks a file to skip
- **`.why.txt`**: Explains why a file was skipped or failed
- **`.orig.mkv`**: Backup of original file after successful transcode

## Intel QSV Hardware Encoding

### Overview

This daemon uses Intel Quick Sync Video (QSV) for hardware-accelerated AV1 encoding. QSV provides significant advantages over VAAPI:

- **10-bit AV1 support**: Full hardware encoding for 10-bit content (HDR, high-quality sources)
- **8-bit AV1 support**: Efficient hardware encoding for standard 8-bit content
- **Better compatibility**: More reliable with Intel Arc GPUs (A310, A380, A770, etc.)
- **Automatic bit depth handling**: Preserves source bit depth in output

### Docker Image Requirement

The daemon requires the specific Docker image:
```
lscr.io/linuxserver/ffmpeg:version-8.0-cli
```

This image includes:
- FFmpeg 8.0 with Intel VPL (Video Processing Library) support
- Intel media drivers with QSV support
- Proper AV1 QSV codec implementation (`av1_qsv`)

The image is automatically pulled by Docker when first used. To manually pull:
```bash
docker pull lscr.io/linuxserver/ffmpeg:version-8.0-cli
```

### Intel Arc GPU Compatibility

**Supported GPUs**:
- Intel Arc A310
- Intel Arc A380
- Intel Arc A580
- Intel Arc A750
- Intel Arc A770

**Requirements**:
- GPU must be accessible via `/dev/dri/renderD128` (or similar render node)
- Intel media drivers installed on host system
- Docker must have access to `/dev/dri` devices

### Encoding Behavior

**8-bit sources** (most web content):
- Input pixel format: yuv420p or similar
- Encoding pixel format: nv12
- Output pixel format: yuv420p
- AV1 profile: main (profile 0)

**10-bit sources** (Blu-ray, HDR content):
- Input pixel format: yuv420p10le or similar
- Encoding pixel format: p010le
- Output pixel format: yuv420p10le
- AV1 profile: main (profile 0)
- HDR metadata preserved

### Quality Settings

The daemon uses the `global_quality` parameter for QSV encoding:
- Range: 20-40 (lower = higher quality)
- Calculated based on source codec, resolution, and bit depth
- Typical values: 28-32 for most content

## Troubleshooting

### QSV Initialization Fails

**Symptom**: Error message about QSV device initialization failure

**Possible causes**:
1. Intel Arc GPU not detected or not accessible
2. Missing Intel media drivers on host system
3. Incorrect device path

**Solutions**:
```bash
# Check if GPU is detected
ls -la /dev/dri/

# Should show renderD128 (or similar)
# Example output:
# crw-rw---- 1 root video 226, 128 Nov 20 10:00 renderD128

# Check GPU info
sudo lspci | grep -i vga

# Verify Intel media driver
vainfo --display drm --device /dev/dri/renderD128

# Check Docker can access device
docker run --rm --device /dev/dri:/dev/dri \
  lscr.io/linuxserver/ffmpeg:version-8.0-cli \
  ffmpeg -hide_banner -init_hw_device qsv=hw:/dev/dri/renderD128
```

### Docker Image Not Found

**Symptom**: Error pulling or running Docker image

**Solutions**:
```bash
# Manually pull the image
docker pull lscr.io/linuxserver/ffmpeg:version-8.0-cli

# Verify image exists
docker images | grep ffmpeg

# Check Docker daemon is running
systemctl status docker
```

### 10-bit Encoding Fails

**Symptom**: 10-bit sources fail to encode or output is 8-bit

**Possible causes**:
1. Using old Docker image without QSV support
2. GPU doesn't support 10-bit AV1 (older Intel GPUs)

**Solutions**:
```bash
# Verify Docker image version
docker inspect lscr.io/linuxserver/ffmpeg:version-8.0-cli | grep -i version

# Test 10-bit encoding manually
docker run --rm --device /dev/dri:/dev/dri \
  -v /path/to/test:/test \
  -e LIBVA_DRIVER_NAME=iHD \
  lscr.io/linuxserver/ffmpeg:version-8.0-cli \
  ffmpeg -init_hw_device qsv=hw:/dev/dri/renderD128 \
    -filter_hw_device hw \
    -i /test/10bit-source.mkv \
    -vf "format=p010le,hwupload" \
    -c:v av1_qsv -global_quality 30 \
    -profile:v main \
    /test/output.mkv

# Check output bit depth
ffprobe -v error -select_streams v:0 \
  -show_entries stream=pix_fmt \
  -of default=noprint_wrappers=1:nokey=1 \
  /test/output.mkv
# Should show: yuv420p10le
```

### Encoding is Slow or Not Using GPU

**Symptom**: Encoding speed is slow, CPU usage high, GPU usage low

**Possible causes**:
1. QSV not properly initialized
2. Wrong driver being used (i965 instead of iHD)
3. Docker not running with proper privileges

**Solutions**:
```bash
# Check GPU utilization during encoding
intel_gpu_top  # Install with: apt install intel-gpu-tools

# Verify iHD driver is being used (check daemon logs)
# Should see: LIBVA_DRIVER_NAME=iHD in Docker command

# Ensure Docker runs with --privileged flag
# (daemon does this automatically)
```

### Permission Denied on /dev/dri

**Symptom**: Cannot access `/dev/dri/renderD128`

**Solutions**:
```bash
# Check permissions
ls -la /dev/dri/renderD128

# Add user to video group
sudo usermod -a -G video $USER

# Or run daemon as root (not recommended for production)
sudo ./av1d --config config.json

# Check Docker has access
docker run --rm --device /dev/dri:/dev/dri \
  alpine ls -la /dev/dri
```

### Logs Show "Cannot load libmfx"

**Symptom**: FFmpeg error about missing libmfx library

**Possible causes**:
1. Using wrong Docker image
2. Docker image corrupted

**Solutions**:
```bash
# Remove old image and re-pull
docker rmi lscr.io/linuxserver/ffmpeg:version-8.0-cli
docker pull lscr.io/linuxserver/ffmpeg:version-8.0-cli

# Verify image has VPL support
docker run --rm lscr.io/linuxserver/ffmpeg:version-8.0-cli \
  ffmpeg -hide_banner -encoders | grep av1_qsv
# Should show: av1_qsv encoder
```

### Output Quality Issues

**Symptom**: Output files have artifacts or quality issues

**Solutions**:
1. Check quality value in logs (should be 20-40)
2. Lower quality value for higher quality (e.g., 25 instead of 32)
3. Verify source file is not corrupted
4. Check if source is already AV1 (daemon should skip these)

**Adjust quality in code** (if needed):
- Edit `crates/daemon/src/classifier.rs`
- Modify `calculate_optimal_qp()` function
- Lower QP values = higher quality, larger files

## Performance Notes

**Typical encoding speeds** (Intel Arc A310):
- 1080p content: 60-120 fps
- 4K content: 20-40 fps
- 10-bit content: Similar to 8-bit (hardware accelerated)

**GPU utilization**:
- Should see 80-100% GPU usage during encoding
- Use `intel_gpu_top` to monitor

**CPU usage**:
- Should be low (10-30%) - most work done on GPU
- High CPU usage indicates software encoding (problem)

