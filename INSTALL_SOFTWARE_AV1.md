# Installation Guide for Software AV1 Encoding

This guide covers installing and configuring the AV1 Daemon for **software-based AV1 encoding** using native FFmpeg 8.0+ with CPU encoders (libsvtav1, libaom-av1, librav1e).

> **Note**: This replaces the previous Docker-based hardware encoding approach. Docker is no longer required.

## Overview

The software AV1 encoding feature uses:
- **FFmpeg 8.0 or later** (native installation, no Docker)
- **CPU-based AV1 encoders**: SVT-AV1, libaom-av1, or librav1e
- **Quality-first approach**: Optimized for maximum perceptual quality
- **Intelligent source classification**: REMUX, WEB-DL, and LOW-QUALITY tiers
- **Test clip workflow**: Validate quality before full encode (REMUX sources)

## Requirements

### System Requirements

**Minimum**:
- FFmpeg 8.0 or later
- At least one AV1 software encoder (libsvtav1, libaom-av1, or librav1e)
- 4+ CPU cores (8+ recommended for reasonable speed)
- 8GB+ RAM (16GB+ recommended for 4K content)
- Rust toolchain (2021 edition or later)

**Recommended**:
- FFmpeg 8.0+ with SVT-AV1-PSY (perceptually-tuned fork)
- 16+ CPU cores for faster encoding
- 32GB+ RAM for 4K content
- Fast NVMe storage for temporary files

### Performance Expectations

Software AV1 encoding is **significantly slower** than hardware encoding:

- **REMUX tier** (preset 3): ~0.5-1 fps on modern CPU (1080p)
- **WEB-DL tier** (preset 5): ~2-4 fps on modern CPU (1080p)
- **LOW-QUALITY tier** (preset 6): ~4-8 fps on modern CPU (1080p)

Expect **10-20x longer encoding times** compared to hardware encoding, but with superior quality preservation.

## FFmpeg 8.0+ Installation

You have three options for installing FFmpeg 8.0+ with AV1 encoder support:

### Option 1: System Package Manager (Easiest)

#### Ubuntu 24.04+ / Debian 13+

```bash
# Update package lists
sudo apt update

# Install FFmpeg (check version first)
apt-cache policy ffmpeg

# If version is 8.0+, install
sudo apt install ffmpeg libsvtav1enc-dev

# Verify installation
ffmpeg -version
ffmpeg -encoders | grep -E "libsvtav1|libaom|librav1e"
```

#### Arch Linux

```bash
# Install FFmpeg with SVT-AV1
sudo pacman -S ffmpeg svt-av1

# Verify installation
ffmpeg -version
ffmpeg -encoders | grep -E "libsvtav1|libaom|librav1e"
```

#### macOS (Homebrew)

```bash
# Install FFmpeg with AV1 encoders
brew install ffmpeg

# Verify installation
ffmpeg -version
ffmpeg -encoders | grep -E "libsvtav1|libaom|librav1e"
```

#### Fedora / RHEL

```bash
# Enable RPM Fusion repositories first
sudo dnf install https://download1.rpmfusion.org/free/fedora/rpmfusion-free-release-$(rpm -E %fedora).noarch.rpm

# Install FFmpeg
sudo dnf install ffmpeg svt-av1

# Verify installation
ffmpeg -version
ffmpeg -encoders | grep -E "libsvtav1|libaom|librav1e"
```

### Option 2: Build from Source (Most Control)

Building from source gives you the latest features and full control over encoder selection.

#### Install Build Dependencies

**Ubuntu/Debian**:
```bash
sudo apt update
sudo apt install -y \
    build-essential \
    yasm \
    nasm \
    cmake \
    git \
    pkg-config \
    libssl-dev \
    ca-certificates
```

**Arch Linux**:
```bash
sudo pacman -S base-devel yasm nasm cmake git
```

**macOS**:
```bash
brew install yasm nasm cmake pkg-config
```

#### Build and Install SVT-AV1

```bash
# Clone SVT-AV1 repository
git clone https://gitlab.com/AOMediaCodec/SVT-AV1.git
cd SVT-AV1

# Build and install
mkdir build && cd build
cmake .. -DCMAKE_BUILD_TYPE=Release
make -j$(nproc)
sudo make install
sudo ldconfig  # Linux only

cd ../..
```

#### Build and Install libaom (Optional, for libaom-av1)

```bash
# Clone libaom repository
git clone https://aomedia.googlesource.com/aom
cd aom

# Build and install
mkdir build && cd build
cmake .. -DCMAKE_BUILD_TYPE=Release -DENABLE_TESTS=0
make -j$(nproc)
sudo make install
sudo ldconfig  # Linux only

cd ../..
```

#### Build and Install librav1e (Optional)

```bash
# Install Rust if not already installed
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env

# Install rav1e
cargo install cargo-c
git clone https://github.com/xiph/rav1e.git
cd rav1e

# Build and install
cargo cinstall --release
sudo ldconfig  # Linux only

cd ..
```

#### Build FFmpeg with AV1 Encoders

```bash
# Clone FFmpeg repository
git clone https://git.ffmpeg.org/ffmpeg.git
cd ffmpeg

# Checkout FFmpeg 8.0 or later
git checkout release/8.0

# Configure with AV1 encoders
./configure \
    --enable-gpl \
    --enable-version3 \
    --enable-libsvtav1 \
    --enable-libaom \
    --enable-librav1e \
    --enable-nonfree \
    --prefix=/usr/local

# Build (this takes a while)
make -j$(nproc)

# Install
sudo make install
sudo ldconfig  # Linux only

# Verify installation
ffmpeg -version
ffmpeg -encoders | grep -E "libsvtav1|libaom|librav1e"
```

Expected output should include:
```
V..... libsvtav1            SVT-AV1(Scalable Video Technology for AV1) encoder (codec av1)
V..... libaom-av1           libaom AV1 (codec av1)
V..... librav1e             librav1e AV1 (codec av1)
```

### Option 3: Static Binary (Quick Start)

Download a pre-built FFmpeg static binary with AV1 encoder support:

#### Linux (x86_64)

```bash
# Download from John Van Sickle's builds
wget https://johnvansickle.com/ffmpeg/releases/ffmpeg-release-amd64-static.tar.xz

# Extract
tar xf ffmpeg-release-amd64-static.tar.xz
cd ffmpeg-*-amd64-static

# Move to a permanent location
sudo mkdir -p /opt/ffmpeg
sudo cp ffmpeg ffprobe /opt/ffmpeg/

# Verify
/opt/ffmpeg/ffmpeg -version
/opt/ffmpeg/ffmpeg -encoders | grep -E "libsvtav1|libaom|librav1e"
```

#### macOS (Universal Binary)

```bash
# Download from evermeet.cx
wget https://evermeet.cx/ffmpeg/ffmpeg-8.0.zip
wget https://evermeet.cx/ffmpeg/ffprobe-8.0.zip

# Extract
unzip ffmpeg-8.0.zip
unzip ffprobe-8.0.zip

# Move to a permanent location
sudo mkdir -p /opt/ffmpeg
sudo mv ffmpeg ffprobe /opt/ffmpeg/

# Verify
/opt/ffmpeg/ffmpeg -version
/opt/ffmpeg/ffmpeg -encoders | grep -E "libsvtav1|libaom|librav1e"
```

## Installing SVT-AV1-PSY (Optional, Recommended)

SVT-AV1-PSY is a perceptually-tuned fork of SVT-AV1 that provides better grain retention and visual quality.

```bash
# Clone SVT-AV1-PSY repository
git clone https://github.com/gianni-rosato/svt-av1-psy.git
cd svt-av1-psy

# Build and install
mkdir build && cd build
cmake .. -DCMAKE_BUILD_TYPE=Release
make -j$(nproc)
sudo make install
sudo ldconfig  # Linux only

cd ../..

# Rebuild FFmpeg with SVT-AV1-PSY
# (Follow FFmpeg build instructions above)
```

## Building the AV1 Daemon

Once FFmpeg 8.0+ is installed with AV1 encoders:

```bash
# Clone or navigate to the project directory
cd /path/to/av1-daemon

# Build the project
cargo build --release

# Binaries will be in target/release/
ls -lh target/release/av1d target/release/av1top
```

## Installation

### Install Binaries

```bash
# Install daemon and TUI
sudo install -m 755 target/release/av1d /usr/local/bin/av1d
sudo install -m 755 target/release/av1top /usr/local/bin/av1top

# Verify installation
which av1d av1top
```

### Create Configuration Directory

```bash
# Create configuration directory
sudo mkdir -p /etc/av1d

# Create job state directory
sudo mkdir -p /var/lib/av1d/jobs
```

### Create Configuration File

Create `/etc/av1d/config.json`:

```json
{
  "library_roots": ["/media/movies", "/media/tv"],
  "min_bytes": 2147483648,
  "max_size_ratio": 0.90,
  "job_state_dir": "/var/lib/av1d/jobs",
  "scan_interval_secs": 60,
  "ffmpeg_bin": "ffmpeg",
  "ffprobe_bin": "ffprobe"
}
```

**Configuration Options**:

- `library_roots`: Array of paths to scan for media files
- `min_bytes`: Minimum file size to process (default: 2GB)
- `max_size_ratio`: Maximum output/input size ratio (default: 0.90)
- `job_state_dir`: Directory for job state persistence
- `scan_interval_secs`: Seconds between library scans
- **`ffmpeg_bin`**: Path to FFmpeg binary (default: `"ffmpeg"`)
- **`ffprobe_bin`**: Path to FFprobe binary (default: `"ffprobe"`)

#### Using Custom FFmpeg Path

If you installed FFmpeg to a custom location (e.g., `/opt/ffmpeg`):

```json
{
  "library_roots": ["/media/movies"],
  "ffmpeg_bin": "/opt/ffmpeg/ffmpeg",
  "ffprobe_bin": "/opt/ffmpeg/ffprobe",
  ...
}
```

#### Using Bundled FFmpeg Binary

If you bundle FFmpeg with your application:

```json
{
  "library_roots": ["/media/movies"],
  "ffmpeg_bin": "/usr/local/lib/av1d/ffmpeg",
  "ffprobe_bin": "/usr/local/lib/av1d/ffprobe",
  ...
}
```

### Create Systemd Service (Optional)

Create `/etc/systemd/system/av1d.service`:

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
CPUQuota=800%  # Limit to 8 cores
MemoryMax=16G  # Limit to 16GB RAM

[Install]
WantedBy=multi-user.target
```

Enable and start the service:

```bash
sudo systemctl daemon-reload
sudo systemctl enable av1d
sudo systemctl start av1d
sudo systemctl status av1d
```

## Verification

### Verify FFmpeg Installation

```bash
# Check FFmpeg version (must be 8.0+)
ffmpeg -version | head -n1

# Check available AV1 encoders
ffmpeg -encoders 2>/dev/null | grep -E "libsvtav1|libaom|librav1e"
```

Expected output:
```
ffmpeg version 8.0 Copyright (c) 2000-2024 the FFmpeg developers
```

And at least one of:
```
V..... libsvtav1            SVT-AV1(Scalable Video Technology for AV1) encoder (codec av1)
V..... libaom-av1           libaom AV1 (codec av1)
V..... librav1e             librav1e AV1 (codec av1)
```

### Verify Daemon Installation

```bash
# Check daemon version
av1d --version

# Check TUI
av1top --version

# Test daemon startup (dry run)
av1d --config /etc/av1d/config.json
```

The daemon should start and log:
```
[INFO] FFmpeg version: 8.0.x
[INFO] Selected AV1 encoder: libsvtav1
[INFO] Starting daemon with 1 library roots
```

Press Ctrl+C to stop.

## Running

### Run as Systemd Service

```bash
# Start service
sudo systemctl start av1d

# Check status
sudo systemctl status av1d

# View logs
sudo journalctl -u av1d -f

# Stop service
sudo systemctl stop av1d
```

### Run Manually

```bash
# Run daemon in foreground
av1d --config /etc/av1d/config.json

# Run in background
nohup av1d --config /etc/av1d/config.json > /var/log/av1d.log 2>&1 &
```

### Monitor with TUI

In another terminal:

```bash
av1top
```

Press `q` to quit, `r` to refresh.

## Troubleshooting

### FFmpeg Version Error

**Symptom**: `FFmpeg 8.0 or later required, found: 7.x`

**Solution**:
```bash
# Check installed version
ffmpeg -version

# If version is too old, install FFmpeg 8.0+ using one of the methods above
# Then verify the daemon finds the correct binary
which ffmpeg
```

### No AV1 Encoders Detected

**Symptom**: `No AV1 software encoders detected. Required: libsvtav1, libaom-av1, or librav1e`

**Solution**:
```bash
# Check available encoders
ffmpeg -encoders 2>/dev/null | grep av1

# If no AV1 encoders are listed, rebuild FFmpeg with encoder support
# See "Build from Source" section above
```

### FFmpeg Not Found

**Symptom**: `FFmpeg binary not found at path: ffmpeg`

**Solution**:
```bash
# Check if ffmpeg is in PATH
which ffmpeg

# If not found, install FFmpeg or specify full path in config
# Edit /etc/av1d/config.json:
{
  "ffmpeg_bin": "/usr/local/bin/ffmpeg",
  "ffprobe_bin": "/usr/local/bin/ffprobe"
}
```

### Encoding is Very Slow

**Expected behavior**: Software AV1 encoding is slow (0.5-4 fps typical).

**Tips to improve speed**:
1. Use faster presets for non-REMUX content (automatic)
2. Ensure SVT-AV1 is installed (fastest software encoder)
3. Increase CPU resources (more cores = faster encoding)
4. Process multiple files in parallel (configure multiple daemon instances)

**Check encoding speed**:
```bash
# Monitor CPU usage
htop

# Check daemon logs for fps
sudo journalctl -u av1d -f | grep fps
```

### Out of Memory Errors

**Symptom**: Encoding fails with OOM errors

**Solution**:
```bash
# Check available memory
free -h

# Reduce concurrent encodes or increase system RAM
# For 4K content, 16GB+ RAM recommended

# Limit daemon memory in systemd service
sudo systemctl edit av1d
# Add: MemoryMax=16G
```

### Test Clip Workflow Hangs

**Symptom**: Daemon pauses waiting for user input on REMUX sources

**Expected behavior**: REMUX sources require test clip approval before full encode.

**Solution**:
```bash
# Check daemon logs for test clip path
sudo journalctl -u av1d -f

# Review test clip quality
mpv /path/to/test-clip.mkv

# Approve or reject via daemon interface
# (Implementation-specific, check daemon documentation)
```

### Permission Denied Errors

**Symptom**: Cannot read/write files in library roots

**Solution**:
```bash
# Check file permissions
ls -la /media/movies

# Ensure daemon user has read/write access
sudo chown -R av1d:av1d /media/movies

# Or run daemon as appropriate user
sudo systemctl edit av1d
# Add: User=your-user
```

## Migration from Docker-Based Encoding

If you're migrating from the previous Docker-based hardware encoding:

### 1. Remove Docker Dependencies

```bash
# Stop old daemon
sudo systemctl stop av1d

# Remove Docker image (optional)
docker rmi lscr.io/linuxserver/ffmpeg:version-8.0-cli
```

### 2. Update Configuration

Edit `/etc/av1d/config.json` and **remove** these fields:
- `docker_image`
- `docker_bin`
- `gpu_device`

**Add** these fields:
- `ffmpeg_bin` (default: `"ffmpeg"`)
- `ffprobe_bin` (default: `"ffprobe"`)

### 3. Install FFmpeg 8.0+

Follow the installation instructions above.

### 4. Rebuild and Reinstall Daemon

```bash
cd /path/to/av1-daemon
cargo build --release
sudo install -m 755 target/release/av1d /usr/local/bin/av1d
```

### 5. Restart Daemon

```bash
sudo systemctl start av1d
sudo systemctl status av1d
```

### Expected Changes

- **Encoding speed**: 10-20x slower (CPU vs GPU)
- **Quality**: Significantly better (quality-first approach)
- **File sizes**: May be larger (quality prioritized over compression)
- **Test clips**: REMUX sources now require user approval
- **Dependencies**: No Docker required

## Performance Tuning

### CPU Affinity

Pin daemon to specific CPU cores for better performance:

```bash
# Edit systemd service
sudo systemctl edit av1d

# Add CPU affinity
[Service]
CPUAffinity=0-15  # Use cores 0-15
```

### Process Priority

Increase daemon priority for faster encoding:

```bash
# Edit systemd service
sudo systemctl edit av1d

# Add nice level
[Service]
Nice=-10  # Higher priority (requires root)
```

### Parallel Encoding

Run multiple daemon instances for parallel encoding:

```bash
# Create separate config files
sudo cp /etc/av1d/config.json /etc/av1d/config-worker2.json

# Edit library_roots to avoid conflicts
# Create separate systemd services
sudo cp /etc/systemd/system/av1d.service /etc/systemd/system/av1d-worker2.service

# Edit ExecStart to use different config
sudo systemctl daemon-reload
sudo systemctl start av1d-worker2
```

## Quality Settings

The daemon automatically selects quality settings based on source classification:

### REMUX Tier (Blu-ray, High-Quality Masters)
- **CRF**: 18 (1080p), 20 (2160p)
- **Preset**: 3 (slower)
- **Film grain**: Enabled (value 8)
- **Test clip**: Required

### WEB-DL Tier (Streaming Downloads)
- **CRF**: 26 (1080p), 28 (2160p)
- **Preset**: 5 (medium)
- **Film grain**: Disabled
- **Test clip**: Skipped

### LOW-QUALITY Tier (Low-Bitrate Rips)
- **CRF**: 30
- **Preset**: 6 (fast)
- **Film grain**: Disabled
- **Test clip**: Skipped

These settings prioritize quality over file size and encoding speed.

## Additional Resources

- **FFmpeg Documentation**: https://ffmpeg.org/documentation.html
- **SVT-AV1 GitHub**: https://gitlab.com/AOMediaCodec/SVT-AV1
- **libaom GitHub**: https://aomedia.googlesource.com/aom
- **librav1e GitHub**: https://github.com/xiph/rav1e
- **AV1 Codec Overview**: https://en.wikipedia.org/wiki/AV1

## Support

For issues or questions:
1. Check daemon logs: `sudo journalctl -u av1d -f`
2. Verify FFmpeg installation: `ffmpeg -version`
3. Check encoder availability: `ffmpeg -encoders | grep av1`
4. Review configuration: `cat /etc/av1d/config.json`

