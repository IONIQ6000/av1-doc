# Quick Start: Software AV1 Encoding

Fast-track installation guide for software AV1 encoding. For detailed instructions, see [INSTALL_SOFTWARE_AV1.md](INSTALL_SOFTWARE_AV1.md).

## Prerequisites

- FFmpeg 8.0+ with AV1 encoder support
- 8+ CPU cores recommended
- 16GB+ RAM for 4K content
- Rust toolchain

## 5-Minute Setup

### 1. Install FFmpeg 8.0+

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

### 2. Verify FFmpeg

```bash
ffmpeg -version | head -n1
ffmpeg -encoders | grep libsvtav1
```

Expected: `ffmpeg version 8.0` and `libsvtav1` encoder listed.

### 3. Build Daemon

```bash
cd /path/to/av1-daemon
cargo build --release
sudo install -m 755 target/release/av1d /usr/local/bin/
sudo install -m 755 target/release/av1top /usr/local/bin/
```

### 4. Create Configuration

```bash
sudo mkdir -p /etc/av1d /var/lib/av1d/jobs

cat | sudo tee /etc/av1d/config.json <<EOF
{
  "library_roots": ["/media/movies"],
  "job_state_dir": "/var/lib/av1d/jobs",
  "ffmpeg_bin": "ffmpeg",
  "ffprobe_bin": "ffprobe"
}
EOF
```

### 5. Run Daemon

```bash
# Test run
av1d --config /etc/av1d/config.json

# Or as systemd service (see below)
```

## Systemd Service (Optional)

```bash
cat | sudo tee /etc/systemd/system/av1d.service <<EOF
[Unit]
Description=AV1 Software Transcoding Daemon
After=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/av1d --config /etc/av1d/config.json
Restart=always
CPUQuota=800%
MemoryMax=16G

[Install]
WantedBy=multi-user.target
EOF

sudo systemctl daemon-reload
sudo systemctl enable av1d
sudo systemctl start av1d
```

## Monitor with TUI

```bash
av1top
```

## Common Scenarios

### Scenario 1: System Package Manager Install

FFmpeg installed via `apt`, `pacman`, or `brew`:

**Config**:
```json
{
  "library_roots": ["/media/movies"],
  "ffmpeg_bin": "ffmpeg"
}
```

**No custom paths needed** - uses system FFmpeg.

### Scenario 2: Custom FFmpeg Location

FFmpeg installed to `/usr/local/bin`:

**Config**:
```json
{
  "library_roots": ["/media/movies"],
  "ffmpeg_bin": "/usr/local/bin/ffmpeg",
  "ffprobe_bin": "/usr/local/bin/ffprobe"
}
```

### Scenario 3: Static Binary

Downloaded static FFmpeg to `/opt/ffmpeg`:

**Config**:
```json
{
  "library_roots": ["/media/movies"],
  "ffmpeg_bin": "/opt/ffmpeg/ffmpeg",
  "ffprobe_bin": "/opt/ffmpeg/ffprobe"
}
```

### Scenario 4: Multiple Library Roots

Scan multiple directories:

**Config**:
```json
{
  "library_roots": ["/media/movies", "/media/tv", "/media/anime"],
  "ffmpeg_bin": "ffmpeg"
}
```

## Troubleshooting

### FFmpeg Not Found

```bash
# Check if installed
which ffmpeg

# If not found, install FFmpeg 8.0+
# See INSTALL_SOFTWARE_AV1.md
```

### Wrong FFmpeg Version

```bash
# Check version
ffmpeg -version

# Must be 8.0+
# Install newer version or build from source
```

### No AV1 Encoders

```bash
# Check encoders
ffmpeg -encoders | grep av1

# If none found, rebuild FFmpeg with encoder support
# See INSTALL_SOFTWARE_AV1.md
```

### Encoding Too Slow

**Expected**: Software encoding is slow (0.5-4 fps).

**Solutions**:
- Add more CPU cores
- Accept slower speed (quality-first approach)
- Process fewer files simultaneously

### Out of Memory

```bash
# Check memory
free -h

# Increase RAM or limit daemon memory
# In systemd service: MemoryMax=16G
```

## Quality Settings

Automatic quality selection based on source:

| Source Type | CRF | Preset | Speed (1080p) |
|-------------|-----|--------|---------------|
| REMUX | 18-20 | 3 (slower) | 0.5-1 fps |
| WEB-DL | 26-28 | 5 (medium) | 2-4 fps |
| LOW-QUALITY | 30 | 6 (fast) | 4-8 fps |

**Lower CRF = Higher Quality = Larger Files**

## Performance Expectations

| Content | Hardware (QSV) | Software (SVT-AV1) |
|---------|----------------|-------------------|
| 1080p REMUX | 60-120 fps | 0.5-1 fps |
| 1080p WEB-DL | 80-150 fps | 2-4 fps |
| 4K REMUX | 20-40 fps | 0.2-0.5 fps |

**Software encoding is 10-20x slower but produces superior quality.**

## Configuration Reference

### Minimal Configuration

```json
{
  "library_roots": ["/media/movies"]
}
```

Uses defaults:
- `ffmpeg_bin`: `"ffmpeg"`
- `ffprobe_bin`: `"ffprobe"`
- `min_bytes`: 2GB
- `max_size_ratio`: 0.90
- `scan_interval_secs`: 60

### Full Configuration

```json
{
  "library_roots": ["/media/movies", "/media/tv"],
  "min_bytes": 2147483648,
  "max_size_ratio": 0.90,
  "job_state_dir": "/var/lib/av1d/jobs",
  "scan_interval_secs": 60,
  "ffmpeg_bin": "/usr/local/bin/ffmpeg",
  "ffprobe_bin": "/usr/local/bin/ffprobe"
}
```

## Next Steps

1. **Monitor first encode**: Use `av1top` to watch progress
2. **Review quality**: Check output files for quality
3. **Adjust settings**: Modify configuration if needed
4. **Read full docs**: See [INSTALL_SOFTWARE_AV1.md](INSTALL_SOFTWARE_AV1.md) for details

## Key Differences from Hardware Encoding

| Aspect | Hardware (Old) | Software (New) |
|--------|---------------|----------------|
| Speed | Fast (60-120 fps) | Slow (0.5-4 fps) |
| Quality | Good | Excellent |
| Docker | Required | Not needed |
| CPU Usage | Low (10-30%) | High (100%) |
| GPU Usage | High (80-100%) | None |

## Migration from Docker

If migrating from Docker-based hardware encoding:

1. **Stop daemon**: `sudo systemctl stop av1d`
2. **Install FFmpeg 8.0+**: See above
3. **Update config**: Remove `docker_image`, `docker_bin`, `gpu_device`
4. **Add FFmpeg paths**: `ffmpeg_bin`, `ffprobe_bin`
5. **Rebuild daemon**: `cargo build --release`
6. **Restart**: `sudo systemctl start av1d`

See [MIGRATION_DOCKER_TO_SOFTWARE.md](MIGRATION_DOCKER_TO_SOFTWARE.md) for detailed migration guide.

## Resources

- **Full Installation Guide**: [INSTALL_SOFTWARE_AV1.md](INSTALL_SOFTWARE_AV1.md)
- **FFmpeg Configuration**: [FFMPEG_CONFIGURATION.md](FFMPEG_CONFIGURATION.md)
- **Migration Guide**: [MIGRATION_DOCKER_TO_SOFTWARE.md](MIGRATION_DOCKER_TO_SOFTWARE.md)
- **FFmpeg Documentation**: https://ffmpeg.org/documentation.html
- **SVT-AV1 GitHub**: https://gitlab.com/AOMediaCodec/SVT-AV1

## Support

Check logs for issues:
```bash
# Systemd service
sudo journalctl -u av1d -f

# Manual run
av1d --config /etc/av1d/config.json
```

Common checks:
```bash
ffmpeg -version                    # Check FFmpeg version
ffmpeg -encoders | grep av1        # Check AV1 encoders
which ffmpeg                       # Find FFmpeg location
cat /etc/av1d/config.json          # Review configuration
```

