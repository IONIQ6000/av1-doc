# Migration Guide: Docker Hardware Encoding → Software AV1 Encoding

This guide helps you migrate from the Docker-based Intel QSV hardware encoding to native software AV1 encoding.

## Overview of Changes

### What's Changing

| Aspect | Docker/Hardware (Old) | Software (New) |
|--------|----------------------|----------------|
| **FFmpeg** | Docker container | Native installation |
| **Encoding** | Intel QSV (GPU) | CPU software encoders |
| **Speed** | Fast (60-120 fps) | Slow (0.5-4 fps) |
| **Quality** | Good | Excellent (quality-first) |
| **Dependencies** | Docker required | No Docker needed |
| **Configuration** | `docker_image`, `gpu_device` | `ffmpeg_bin`, `ffprobe_bin` |

### Why Migrate?

**Advantages of Software Encoding**:
- **Superior quality**: Quality-first approach preserves grain, texture, and detail
- **No Docker dependency**: Simpler deployment and maintenance
- **Better source handling**: Intelligent classification (REMUX, WEB-DL, LOW-QUALITY)
- **Test clip workflow**: Validate quality before full encode (REMUX sources)
- **Perceptual tuning**: SVT-AV1-PSY support for grain-optimized encoding

**Trade-offs**:
- **Much slower**: 10-20x slower encoding (CPU vs GPU)
- **Higher CPU usage**: 100% CPU utilization during encoding
- **More power consumption**: CPU encoding uses more power than GPU

## Migration Steps

### Step 1: Backup Current Configuration

```bash
# Backup configuration
sudo cp /etc/av1d/config.json /etc/av1d/config.json.backup

# Backup job state
sudo cp -r /var/lib/av1d/jobs /var/lib/av1d/jobs.backup

# Stop daemon
sudo systemctl stop av1d
```

### Step 2: Install FFmpeg 8.0+ with AV1 Encoders

Follow the [Software AV1 Installation Guide](INSTALL_SOFTWARE_AV1.md) to install FFmpeg 8.0+ with at least one AV1 encoder.

**Quick verification**:
```bash
# Check FFmpeg version (must be 8.0+)
ffmpeg -version | head -n1

# Check AV1 encoders (need at least one)
ffmpeg -encoders 2>/dev/null | grep -E "libsvtav1|libaom|librav1e"
```

Expected output:
```
ffmpeg version 8.0 Copyright (c) 2000-2024 the FFmpeg developers
V..... libsvtav1            SVT-AV1(Scalable Video Technology for AV1) encoder (codec av1)
```

### Step 3: Update Configuration File

Edit `/etc/av1d/config.json` to remove Docker settings and add FFmpeg paths.

**Old Configuration** (Docker-based):
```json
{
  "library_roots": ["/media/movies", "/media/tv"],
  "min_bytes": 2147483648,
  "max_size_ratio": 0.90,
  "job_state_dir": "/var/lib/av1d/jobs",
  "scan_interval_secs": 60,
  "docker_image": "lscr.io/linuxserver/ffmpeg:version-8.0-cli",
  "docker_bin": "docker",
  "gpu_device": "/dev/dri"
}
```

**New Configuration** (Software-based):
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

**Changes**:
- ❌ **Remove**: `docker_image`
- ❌ **Remove**: `docker_bin`
- ❌ **Remove**: `gpu_device`
- ✅ **Add**: `ffmpeg_bin` (default: `"ffmpeg"`)
- ✅ **Add**: `ffprobe_bin` (default: `"ffprobe"`)

If FFmpeg is installed to a custom location:
```json
{
  "ffmpeg_bin": "/usr/local/bin/ffmpeg",
  "ffprobe_bin": "/usr/local/bin/ffprobe"
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
```

### Step 5: Update Systemd Service (Optional)

Edit `/etc/systemd/system/av1d.service` to remove Docker dependency and add resource limits.

**Old Service** (Docker-based):
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

**New Service** (Software-based):
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
CPUQuota=800%  # Limit to 8 cores (800% = 8 cores)
MemoryMax=16G  # Limit to 16GB RAM

[Install]
WantedBy=multi-user.target
```

**Changes**:
- ❌ **Remove**: `After=docker.service`
- ❌ **Remove**: `Requires=docker.service`
- ✅ **Add**: `CPUQuota` to limit CPU usage
- ✅ **Add**: `MemoryMax` to limit memory usage

Reload systemd:
```bash
sudo systemctl daemon-reload
```

### Step 6: Test Configuration

Test the daemon before starting as a service:

```bash
# Run daemon in foreground
av1d --config /etc/av1d/config.json
```

Expected output:
```
[INFO] FFmpeg version: 8.0.x
[INFO] Available AV1 encoders: libsvtav1
[INFO] Selected AV1 encoder: libsvtav1
[INFO] Starting daemon with 2 library roots
[INFO] Scanning /media/movies...
```

Press Ctrl+C to stop.

If you see errors, check the [Troubleshooting](#troubleshooting) section.

### Step 7: Start Daemon

```bash
# Start daemon service
sudo systemctl start av1d

# Check status
sudo systemctl status av1d

# View logs
sudo journalctl -u av1d -f
```

### Step 8: Monitor First Encode

Use the TUI to monitor the first encode:

```bash
av1top
```

**What to expect**:
- **Much slower encoding**: 0.5-4 fps (vs 60-120 fps with hardware)
- **High CPU usage**: 100% CPU utilization
- **Better quality**: Larger files with superior visual quality
- **Test clip workflow**: REMUX sources will pause for user approval

### Step 9: Clean Up Docker (Optional)

Once you've verified software encoding works:

```bash
# Remove Docker image (optional)
docker rmi lscr.io/linuxserver/ffmpeg:version-8.0-cli

# Uninstall Docker (optional, if not used for other purposes)
sudo apt remove docker.io
```

## Configuration Comparison

### Complete Before/After Example

**Before** (Docker/Hardware):
```json
{
  "library_roots": ["/media/movies", "/media/tv"],
  "min_bytes": 2147483648,
  "max_size_ratio": 0.90,
  "job_state_dir": "/var/lib/av1d/jobs",
  "scan_interval_secs": 60,
  "docker_image": "lscr.io/linuxserver/ffmpeg:version-8.0-cli",
  "docker_bin": "docker",
  "gpu_device": "/dev/dri"
}
```

**After** (Software):
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

## Expected Behavior Changes

### Encoding Speed

| Content Type | Hardware (QSV) | Software (SVT-AV1) | Slowdown |
|--------------|----------------|-------------------|----------|
| 1080p REMUX | 60-120 fps | 0.5-1 fps | 60-120x |
| 1080p WEB-DL | 80-150 fps | 2-4 fps | 20-40x |
| 4K REMUX | 20-40 fps | 0.2-0.5 fps | 40-100x |

**Example**: A 2-hour 1080p movie that took 2 minutes with hardware will now take 2-4 hours with software.

### Quality Improvements

**REMUX sources**:
- Better grain preservation
- No banding in gradients
- Sharper detail retention
- Film-grain synthesis enabled
- Test clip validation before full encode

**WEB-DL sources**:
- Conservative re-encoding (or skipped if already modern codec)
- No artifact compounding
- Appropriate CRF selection

**LOW-QUALITY sources**:
- Size-optimized encoding
- Fast presets for already-degraded content

### File Sizes

**Hardware encoding** (QSV):
- Aggressive compression
- Smaller files (50-70% of original)

**Software encoding** (Quality-first):
- Quality-prioritized compression
- Larger files (60-90% of original for REMUX)
- Smaller files for WEB-DL/LOW-QUALITY

### CPU and Power Usage

**Hardware encoding**:
- Low CPU usage (10-30%)
- GPU usage (80-100%)
- Lower power consumption

**Software encoding**:
- High CPU usage (100%)
- No GPU usage
- Higher power consumption

## Troubleshooting

### Daemon Won't Start

**Error**: `FFmpeg 8.0 or later required, found: 7.x`

**Solution**:
```bash
# Check FFmpeg version
ffmpeg -version

# Install FFmpeg 8.0+ (see INSTALL_SOFTWARE_AV1.md)
```

---

**Error**: `No AV1 software encoders detected`

**Solution**:
```bash
# Check available encoders
ffmpeg -encoders | grep av1

# If none found, rebuild FFmpeg with encoder support
# See INSTALL_SOFTWARE_AV1.md for build instructions
```

---

**Error**: `FFmpeg binary not found at path: ffmpeg`

**Solution**:
```bash
# Find FFmpeg location
which ffmpeg

# Update config with full path
{
  "ffmpeg_bin": "/usr/local/bin/ffmpeg"
}
```

### Encoding is Too Slow

**Expected behavior**: Software encoding is 10-20x slower than hardware.

**Options**:
1. **Accept slower speed**: Quality-first approach prioritizes quality over speed
2. **Add more CPU cores**: Encoding scales with CPU cores
3. **Process fewer files**: Reduce library scan frequency
4. **Use faster presets**: Edit quality calculator (not recommended, reduces quality)

### Out of Memory

**Error**: Encoding fails with OOM errors

**Solution**:
```bash
# Check available memory
free -h

# Increase system RAM (16GB+ recommended for 4K)
# Or limit daemon memory in systemd service:
sudo systemctl edit av1d
# Add: MemoryMax=16G
```

### Test Clip Workflow Confusion

**Behavior**: Daemon pauses on REMUX sources waiting for input

**Expected**: REMUX sources require test clip approval before full encode.

**Solution**:
1. Check daemon logs for test clip path
2. Review test clip quality
3. Approve or request adjustment
4. Daemon will proceed with full encode

### Want to Revert to Hardware Encoding

If you need to revert:

```bash
# Stop daemon
sudo systemctl stop av1d

# Restore backup configuration
sudo cp /etc/av1d/config.json.backup /etc/av1d/config.json

# Restore old daemon binary (if you kept it)
# Or rebuild from old commit

# Restore systemd service
sudo systemctl daemon-reload

# Start daemon
sudo systemctl start av1d
```

## Performance Tuning

### Limit CPU Usage

Prevent daemon from using all CPU cores:

```bash
# Edit systemd service
sudo systemctl edit av1d

# Add CPU limit (400% = 4 cores)
[Service]
CPUQuota=400%
```

### Increase Process Priority

Give daemon higher priority for faster encoding:

```bash
# Edit systemd service
sudo systemctl edit av1d

# Add nice level (requires root)
[Service]
Nice=-10
```

### Parallel Encoding

Run multiple daemon instances for parallel encoding:

```bash
# Create separate configs for different library roots
sudo cp /etc/av1d/config.json /etc/av1d/config-movies.json
sudo cp /etc/av1d/config.json /etc/av1d/config-tv.json

# Edit each config to process different directories
# Create separate systemd services
sudo cp /etc/systemd/system/av1d.service /etc/systemd/system/av1d-movies.service
sudo cp /etc/systemd/system/av1d.service /etc/systemd/system/av1d-tv.service

# Edit ExecStart to use different configs
sudo systemctl daemon-reload
sudo systemctl start av1d-movies av1d-tv
```

## Rollback Plan

If you encounter issues and need to rollback:

### Quick Rollback

```bash
# Stop new daemon
sudo systemctl stop av1d

# Restore backup configuration
sudo cp /etc/av1d/config.json.backup /etc/av1d/config.json

# Restore old daemon binary (if available)
# Or rebuild from previous version

# Start daemon
sudo systemctl start av1d
```

### Full Rollback

```bash
# Stop daemon
sudo systemctl stop av1d

# Restore configuration
sudo cp /etc/av1d/config.json.backup /etc/av1d/config.json

# Restore job state
sudo rm -rf /var/lib/av1d/jobs
sudo cp -r /var/lib/av1d/jobs.backup /var/lib/av1d/jobs

# Reinstall Docker (if removed)
sudo apt install docker.io

# Pull Docker image
docker pull lscr.io/linuxserver/ffmpeg:version-8.0-cli

# Restore old daemon binary
# (rebuild from old commit or restore from backup)

# Restore systemd service
sudo systemctl daemon-reload
sudo systemctl start av1d
```

## FAQ

### Q: Can I use both hardware and software encoding?

**A**: Not simultaneously in the same daemon instance. You would need to run two separate daemon instances with different configurations.

### Q: Will my existing job state work?

**A**: Yes, job state files are compatible. Pending jobs will be processed with the new software encoding.

### Q: Can I speed up software encoding?

**A**: Yes, but at the cost of quality:
- Use faster presets (not recommended for REMUX)
- Increase CRF values (lower quality)
- Skip test clip workflow (not recommended)

The quality-first approach intentionally prioritizes quality over speed.

### Q: What about 10-bit and HDR content?

**A**: Fully supported. Software encoding preserves 10-bit color depth and HDR metadata, just like hardware encoding.

### Q: Should I migrate?

**A**: Depends on your priorities:
- **Migrate if**: Quality is paramount, you have CPU resources, you want to eliminate Docker
- **Stay on hardware if**: Speed is critical, you have limited CPU, you need high throughput

### Q: Can I test software encoding before fully migrating?

**A**: Yes! Run software encoding on a test directory:

```bash
# Create test config
cat > /tmp/test-config.json <<EOF
{
  "library_roots": ["/tmp/test-media"],
  "job_state_dir": "/tmp/test-jobs",
  "ffmpeg_bin": "ffmpeg",
  "ffprobe_bin": "ffprobe"
}
EOF

# Run daemon with test config
av1d --config /tmp/test-config.json
```

## Additional Resources

- [Software AV1 Installation Guide](INSTALL_SOFTWARE_AV1.md) - Full installation instructions
- [FFmpeg Configuration Guide](FFMPEG_CONFIGURATION.md) - FFmpeg binary configuration
- [Quality Settings Documentation](INSTALL_SOFTWARE_AV1.md#quality-settings) - Understanding quality tiers

## Support

If you encounter issues during migration:

1. Check daemon logs: `sudo journalctl -u av1d -f`
2. Verify FFmpeg: `ffmpeg -version`
3. Check encoders: `ffmpeg -encoders | grep av1`
4. Review configuration: `cat /etc/av1d/config.json`
5. Test manually: `av1d --config /etc/av1d/config.json`

