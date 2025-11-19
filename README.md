# AV1 Daemon (Rust)

A Rust-based AV1 transcoding daemon with ratatui TUI, using Docker ffmpeg with VAAPI AV1 hardware acceleration.

## Overview

This project provides:
- **`av1d`**: A daemon that watches media libraries and transcodes files to AV1 using VAAPI hardware acceleration
- **`av1top`**: A ratatui-based terminal UI for monitoring transcoding jobs

## Architecture

The project is organized as a Cargo workspace with three crates:

- **`crates/daemon`**: Core library with config, job management, scanning, ffprobe/ffmpeg wrappers, and classification
- **`crates/cli-daemon`**: Binary crate for the `av1d` daemon
- **`crates/cli-tui`**: Binary crate for the `av1top` monitoring TUI

## Requirements

- Rust toolchain (2021 edition)
- Docker installed and running
- Intel GPU with VAAPI support (e.g., Arc A310)
- `/dev/dri` device available for GPU passthrough

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
5. Process jobs: probe, classify, transcode to AV1 VAAPI
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

- **Docker-based ffmpeg**: Uses `lscr.io/linuxserver/ffmpeg:version-8.0-cli` image
- **VAAPI AV1**: Hardware-accelerated AV1 encoding via Intel GPU
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

