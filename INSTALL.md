# Installation Guide for Debian 13 Trixie

This guide covers installing the AV1 Daemon on Debian 13 Trixie containers.

## Quick Install

Run the install script as root:

```bash
sudo ./install.sh
```

This will:
1. Install system dependencies (build tools, Docker, etc.)
2. Install Rust toolchain
3. Build the project
4. Install binaries to `/usr/local/bin`
5. Create systemd service file
6. Create default configuration

## Manual Installation

### 1. Install System Dependencies

```bash
apt-get update
apt-get install -y \
    curl \
    build-essential \
    pkg-config \
    libssl-dev \
    ca-certificates \
    git \
    docker.io
```

### 2. Install Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source $HOME/.cargo/env
```

### 3. Build the Project

```bash
cd /path/to/av1-doc
cargo build --release
```

### 4. Install Binaries

```bash
install -m 755 target/release/av1d /usr/local/bin/av1d
install -m 755 target/release/av1top /usr/local/bin/av1top
```

### 5. Create Configuration

```bash
mkdir -p /etc/av1d
cat > /etc/av1d/config.json <<EOF
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
EOF

mkdir -p /var/lib/av1d/jobs
```

### 6. Pull Docker Image

```bash
docker pull lscr.io/linuxserver/ffmpeg:version-8.0-cli
```

### 7. Create Systemd Service (Optional)

```bash
cat > /etc/systemd/system/av1d.service <<EOF
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
EOF

systemctl daemon-reload
systemctl enable av1d
systemctl start av1d
```

## Docker Container Installation

If you're installing inside a Docker container, you may need to:

1. **Mount GPU device** (if using VAAPI):
   ```bash
   docker run --device=/dev/dri:/dev/dri ...
   ```

2. **Mount library directories**:
   ```bash
   docker run -v /media:/media ...
   ```

3. **Use host Docker socket** (if daemon runs in container but needs Docker):
   ```bash
   docker run -v /var/run/docker.sock:/var/run/docker.sock ...
   ```

## Configuration

Edit `/etc/av1d/config.json` to set:
- `library_roots`: Paths to your media library
- `min_bytes`: Minimum file size to process (default: 2GB)
- `max_size_ratio`: Maximum size ratio for acceptance (default: 0.90)
- `job_state_dir`: Where job JSON files are stored
- `scan_interval_secs`: How often to scan for new files

## Running

### As a Service

```bash
systemctl start av1d
systemctl status av1d
journalctl -u av1d -f
```

### Manually

```bash
av1d --config /etc/av1d/config.json
```

### Monitor with TUI

In another terminal:
```bash
av1top
```

## Requirements

- Debian 13 Trixie (or compatible)
- Docker installed and running
- Intel GPU with VAAPI support (for hardware acceleration)
- `/dev/dri` device accessible (for GPU passthrough)
- Sufficient disk space for transcoded files

## Troubleshooting

### Docker not accessible
Ensure Docker is running:
```bash
systemctl status docker
```

### GPU not accessible
Check `/dev/dri` exists:
```bash
ls -la /dev/dri/
```

### Permission denied
Ensure user has access to Docker socket:
```bash
usermod -aG docker $USER
```

### Build fails
Ensure Rust toolchain is up to date:
```bash
rustup update stable
```

