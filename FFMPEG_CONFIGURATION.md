# FFmpeg Configuration Guide

This guide explains how to configure FFmpeg binary paths for the AV1 Daemon software encoding feature.

## Configuration Options

The daemon uses two configuration fields to locate FFmpeg binaries:

- **`ffmpeg_bin`**: Path to the FFmpeg executable
- **`ffprobe_bin`**: Path to the FFprobe executable

## Default Behavior

If not specified, the daemon uses default values:

```json
{
  "ffmpeg_bin": "ffmpeg",
  "ffprobe_bin": "ffprobe"
}
```

This searches for `ffmpeg` and `ffprobe` in your system's `PATH` environment variable.

## Configuration Examples

### Example 1: System Installation (Default)

FFmpeg installed via package manager (e.g., `apt install ffmpeg`):

```json
{
  "library_roots": ["/media/movies"],
  "job_state_dir": "/var/lib/av1d/jobs"
}
```

No need to specify `ffmpeg_bin` or `ffprobe_bin` - defaults will work.

### Example 2: Custom Installation Path

FFmpeg installed to `/usr/local/bin`:

```json
{
  "library_roots": ["/media/movies"],
  "ffmpeg_bin": "/usr/local/bin/ffmpeg",
  "ffprobe_bin": "/usr/local/bin/ffprobe",
  "job_state_dir": "/var/lib/av1d/jobs"
}
```

### Example 3: Static Binary in /opt

Downloaded static FFmpeg binary to `/opt/ffmpeg`:

```json
{
  "library_roots": ["/media/movies"],
  "ffmpeg_bin": "/opt/ffmpeg/ffmpeg",
  "ffprobe_bin": "/opt/ffmpeg/ffprobe",
  "job_state_dir": "/var/lib/av1d/jobs"
}
```

### Example 4: Bundled with Application

FFmpeg bundled with the daemon in `/usr/local/lib/av1d`:

```json
{
  "library_roots": ["/media/movies"],
  "ffmpeg_bin": "/usr/local/lib/av1d/bin/ffmpeg",
  "ffprobe_bin": "/usr/local/lib/av1d/bin/ffprobe",
  "job_state_dir": "/var/lib/av1d/jobs"
}
```

### Example 5: User-Specific Installation

FFmpeg installed in user's home directory:

```json
{
  "library_roots": ["/media/movies"],
  "ffmpeg_bin": "/home/username/.local/bin/ffmpeg",
  "ffprobe_bin": "/home/username/.local/bin/ffprobe",
  "job_state_dir": "/var/lib/av1d/jobs"
}
```

### Example 6: Multiple FFmpeg Versions

Using a specific FFmpeg version when multiple are installed:

```bash
# System FFmpeg (older version)
/usr/bin/ffmpeg -> version 7.0

# Custom FFmpeg (newer version)
/opt/ffmpeg-8.0/ffmpeg -> version 8.0
```

Configuration:

```json
{
  "library_roots": ["/media/movies"],
  "ffmpeg_bin": "/opt/ffmpeg-8.0/ffmpeg",
  "ffprobe_bin": "/opt/ffmpeg-8.0/ffprobe",
  "job_state_dir": "/var/lib/av1d/jobs"
}
```

## Verification

### Check FFmpeg Location

```bash
# Find FFmpeg in PATH
which ffmpeg

# Check version
ffmpeg -version

# Check available encoders
ffmpeg -encoders | grep -E "libsvtav1|libaom|librav1e"
```

### Test Configuration

```bash
# Test with specific binary
/path/to/ffmpeg -version

# Test encoder availability
/path/to/ffmpeg -encoders | grep av1
```

### Verify Daemon Configuration

```bash
# Start daemon with config
av1d --config /etc/av1d/config.json

# Check logs for FFmpeg version detection
# Should see:
# [INFO] FFmpeg version: 8.0.x
# [INFO] Selected AV1 encoder: libsvtav1
```

## Troubleshooting

### Error: "FFmpeg binary not found"

**Cause**: The specified path doesn't exist or isn't executable.

**Solution**:
```bash
# Check if file exists
ls -la /path/to/ffmpeg

# Check if executable
file /path/to/ffmpeg

# Make executable if needed
chmod +x /path/to/ffmpeg

# Verify it runs
/path/to/ffmpeg -version
```

### Error: "FFmpeg 8.0 or later required, found: 7.x"

**Cause**: The FFmpeg binary is too old.

**Solution**:
```bash
# Check version
ffmpeg -version

# Install FFmpeg 8.0+ (see INSTALL_SOFTWARE_AV1.md)
# Then update config to point to new binary
```

### Error: "No AV1 software encoders detected"

**Cause**: FFmpeg was built without AV1 encoder support.

**Solution**:
```bash
# Check available encoders
ffmpeg -encoders | grep av1

# If no AV1 encoders listed, rebuild FFmpeg with encoder support
# See INSTALL_SOFTWARE_AV1.md for build instructions
```

### FFmpeg Found but Wrong Version Used

**Cause**: Multiple FFmpeg installations, daemon using wrong one.

**Solution**:
```bash
# Find all FFmpeg installations
which -a ffmpeg

# Check each version
/usr/bin/ffmpeg -version
/usr/local/bin/ffmpeg -version

# Specify correct path in config
{
  "ffmpeg_bin": "/usr/local/bin/ffmpeg"
}
```

## Environment Variables

The daemon respects the `PATH` environment variable when using default configuration.

### Modify PATH for Systemd Service

Edit `/etc/systemd/system/av1d.service`:

```ini
[Service]
Environment="PATH=/opt/ffmpeg/bin:/usr/local/bin:/usr/bin:/bin"
ExecStart=/usr/local/bin/av1d --config /etc/av1d/config.json
```

### Modify PATH for Manual Execution

```bash
# Temporary (current session)
export PATH="/opt/ffmpeg/bin:$PATH"
av1d --config /etc/av1d/config.json

# Permanent (add to ~/.bashrc or ~/.zshrc)
echo 'export PATH="/opt/ffmpeg/bin:$PATH"' >> ~/.bashrc
source ~/.bashrc
```

## Bundled Binary Deployment

If you want to bundle FFmpeg with your application:

### Directory Structure

```
/usr/local/lib/av1d/
├── bin/
│   ├── ffmpeg
│   └── ffprobe
└── lib/
    ├── libsvtav1.so
    └── ... (other libraries)
```

### Configuration

```json
{
  "ffmpeg_bin": "/usr/local/lib/av1d/bin/ffmpeg",
  "ffprobe_bin": "/usr/local/lib/av1d/bin/ffprobe"
}
```

### Library Path

If using bundled libraries, set `LD_LIBRARY_PATH`:

```bash
# In systemd service
[Service]
Environment="LD_LIBRARY_PATH=/usr/local/lib/av1d/lib"
ExecStart=/usr/local/bin/av1d --config /etc/av1d/config.json
```

### Static Binary (Recommended for Bundling)

Use a static FFmpeg binary to avoid library dependencies:

```bash
# Download static binary
wget https://johnvansickle.com/ffmpeg/releases/ffmpeg-release-amd64-static.tar.xz
tar xf ffmpeg-release-amd64-static.tar.xz

# Copy to bundle directory
mkdir -p /usr/local/lib/av1d/bin
cp ffmpeg-*-amd64-static/ffmpeg /usr/local/lib/av1d/bin/
cp ffmpeg-*-amd64-static/ffprobe /usr/local/lib/av1d/bin/

# No LD_LIBRARY_PATH needed for static binaries
```

## Best Practices

1. **Use absolute paths** in configuration for production deployments
2. **Verify FFmpeg version** before starting daemon (≥8.0 required)
3. **Check encoder availability** to ensure at least one AV1 encoder is present
4. **Use static binaries** for bundled deployments to avoid library conflicts
5. **Document custom paths** in deployment documentation
6. **Test configuration** before deploying to production

## Related Documentation

- [Software AV1 Installation Guide](INSTALL_SOFTWARE_AV1.md) - Full installation instructions
- [Configuration Reference](README.md) - All configuration options
- [Troubleshooting Guide](INSTALL_SOFTWARE_AV1.md#troubleshooting) - Common issues and solutions

