#!/usr/bin/env bash
set -euo pipefail

# remove_docker_and_switch_to_local_ffmpeg.sh
# WARNING: This removes Docker packages, images, containers, volumes, and data.
# Make sure you don't need any Docker-stored data before running.

echo "==> Stopping running Docker services (if any)..."
if command -v systemctl >/dev/null 2>&1; then
  sudo systemctl stop docker || true
  sudo systemctl stop containerd || true
fi

echo "==> Removing Docker containers/images/volumes/networks..."
if command -v docker >/dev/null 2>&1; then
  sudo docker system prune -a --volumes -f || true
fi

echo "==> Uninstalling Docker packages..."
if command -v apt-get >/dev/null 2>&1; then
  sudo apt-get purge -y docker-engine docker docker.io docker-ce docker-ce-cli docker-compose-plugin docker-compose || true
  sudo apt-get autoremove -y --purge docker-engine docker docker.io docker-ce docker-compose-plugin docker-compose || true
elif command -v dnf >/dev/null 2>&1; then
  sudo dnf remove -y docker docker-ce docker-ce-cli docker-compose-plugin containerd.io || true
elif command -v yum >/dev/null 2>&1; then
  sudo yum remove -y docker docker-ce docker-ce-cli docker-compose-plugin containerd.io || true
elif command -v pacman >/dev/null 2>&1; then
  sudo pacman -Rns --noconfirm docker docker-compose containerd || true
else
  echo "No supported package manager found. Remove Docker manually."
fi

echo "==> Deleting Docker data directories..."
sudo rm -rf /var/lib/docker /var/lib/containerd /etc/docker /run/docker.sock || true

echo "==> Removing docker group (optional)..."
if getent group docker >/dev/null 2>&1; then
  sudo groupdel docker || true
fi

echo "==> Verifying Docker is gone..."
if command -v docker >/dev/null 2>&1; then
  echo "WARNING: docker command still exists in PATH."
else
  echo "Docker removed."
fi

echo "==> Verifying local FFmpeg..."
if ! command -v ffmpeg >/dev/null 2>&1; then
  echo "ERROR: ffmpeg not found in PATH. Install/bundle FFmpeg >= 8.0 and re-run."
  exit 1
fi

FFVER=$(ffmpeg -version | head -n1 | awk '{print $3}')
echo "Found ffmpeg version: $FFVER"
# naive semver compare: ensure major >= 8
MAJOR=${FFVER%%.*}
if [[ "$MAJOR" -lt 8 ]]; then
  echo "ERROR: FFmpeg is < 8.0. Install FFmpeg 8.0+."
  exit 1
fi

echo "==> Verifying FFprobe..."
if ! command -v ffprobe >/dev/null 2>&1; then
  echo "ERROR: ffprobe not found in PATH. Install FFmpeg with ffprobe."
  exit 1
fi
echo "Found ffprobe: $(command -v ffprobe)"

echo "==> Checking for AV1 encoders..."
ENCODERS=$(ffmpeg -hide_banner -encoders 2>/dev/null)

# Check for software AV1 encoders
SOFTWARE_AV1=""
if echo "$ENCODERS" | grep -q "libsvtav1"; then
  SOFTWARE_AV1="libsvtav1"
  echo "  ✓ Found libsvtav1 (software encoder)"
fi
if echo "$ENCODERS" | grep -q "libaom-av1"; then
  SOFTWARE_AV1="${SOFTWARE_AV1:+$SOFTWARE_AV1, }libaom-av1"
  echo "  ✓ Found libaom-av1 (software encoder)"
fi
if echo "$ENCODERS" | grep -q "librav1e"; then
  SOFTWARE_AV1="${SOFTWARE_AV1:+$SOFTWARE_AV1, }librav1e"
  echo "  ✓ Found librav1e (software encoder)"
fi

# Check for hardware AV1 encoder (QSV)
HARDWARE_AV1=""
if echo "$ENCODERS" | grep -q "av1_qsv"; then
  HARDWARE_AV1="av1_qsv"
  echo "  ✓ Found av1_qsv (Intel QSV hardware encoder)"
fi

# Validate at least one encoder is available
if [[ -z "$SOFTWARE_AV1" && -z "$HARDWARE_AV1" ]]; then
  echo "ERROR: No AV1 encoders found (software or hardware)."
  echo "       Software encoders: libsvtav1, libaom-av1, librav1e"
  echo "       Hardware encoders: av1_qsv (Intel QSV)"
  echo "       Rebuild FFmpeg with --enable-libsvtav1 or ensure QSV drivers are installed."
  exit 1
fi

if [[ -z "$SOFTWARE_AV1" ]]; then
  echo "WARNING: No software AV1 encoders found. Only hardware encoding available."
  echo "         Consider rebuilding FFmpeg with --enable-libsvtav1 for software encoding."
fi

echo ""
echo "==> Summary:"
echo "  Docker: Removed"
echo "  FFmpeg: $FFVER"
echo "  FFprobe: Available"
if [[ -n "$SOFTWARE_AV1" ]]; then
  echo "  Software AV1: $SOFTWARE_AV1"
fi
if [[ -n "$HARDWARE_AV1" ]]; then
  echo "  Hardware AV1: $HARDWARE_AV1"
fi
echo ""
echo "==> Done. Update your app/service to call ffmpeg directly (FFMPEG_BIN=ffmpeg)."
