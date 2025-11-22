#!/usr/bin/env bash
set -euo pipefail

# migrate_config.sh
# Automatically migrates av1d config from Docker-based to native FFmpeg

CONFIG_FILE="${1:-/etc/av1d/config.json}"
BACKUP_FILE="${CONFIG_FILE}.pre-migration-$(date +%Y%m%d-%H%M%S)"

echo "==> Migrating config: $CONFIG_FILE"

# Check if config exists
if [[ ! -f "$CONFIG_FILE" ]]; then
  echo "ERROR: Config file not found: $CONFIG_FILE"
  echo "Usage: $0 [config-file-path]"
  echo "Example: $0 /etc/av1d/config.json"
  exit 1
fi

# Backup original config
echo "==> Creating backup: $BACKUP_FILE"
sudo cp "$CONFIG_FILE" "$BACKUP_FILE"

# Check if already migrated
if grep -q '"ffmpeg_bin"' "$CONFIG_FILE"; then
  echo "WARNING: Config appears to already have ffmpeg_bin. Already migrated?"
  read -p "Continue anyway? (y/N): " -n 1 -r
  echo
  if [[ ! $REPLY =~ ^[Yy]$ ]]; then
    echo "Aborted."
    exit 0
  fi
fi

# Create temporary file for new config
TEMP_FILE=$(mktemp)

# Use jq to transform the config (if available)
if command -v jq >/dev/null 2>&1; then
  echo "==> Using jq to transform config..."
  
  sudo jq '
    # Remove Docker-related fields
    del(.docker_image, .docker_bin, .gpu_device) |
    
    # Add FFmpeg fields (only if not already present)
    if has("ffmpeg_bin") then . else . + {"ffmpeg_bin": "ffmpeg"} end |
    if has("ffprobe_bin") then . else . + {"ffprobe_bin": "ffprobe"} end
  ' "$CONFIG_FILE" > "$TEMP_FILE"
  
  # Move transformed config back
  sudo mv "$TEMP_FILE" "$CONFIG_FILE"
  
  echo "==> Config migrated successfully using jq"
  
else
  # Fallback: Use sed for simple transformation
  echo "==> jq not found, using sed for transformation..."
  echo "WARNING: sed method is less robust. Consider installing jq: sudo apt install jq"
  
  # Copy to temp file
  sudo cp "$CONFIG_FILE" "$TEMP_FILE"
  
  # Remove Docker fields (remove entire lines)
  sudo sed -i '/"docker_image"/d' "$TEMP_FILE"
  sudo sed -i '/"docker_bin"/d' "$TEMP_FILE"
  sudo sed -i '/"gpu_device"/d' "$TEMP_FILE"
  
  # Check if ffmpeg_bin already exists
  if ! grep -q '"ffmpeg_bin"' "$TEMP_FILE"; then
    # Add ffmpeg_bin and ffprobe_bin before the closing brace
    # This is a bit hacky but works for simple configs
    sudo sed -i 's/}$/,\n  "ffmpeg_bin": "ffmpeg",\n  "ffprobe_bin": "ffprobe"\n}/' "$TEMP_FILE"
  fi
  
  # Move transformed config back
  sudo mv "$TEMP_FILE" "$CONFIG_FILE"
  
  echo "==> Config migrated successfully using sed"
fi

# Validate JSON syntax
if command -v jq >/dev/null 2>&1; then
  if ! jq empty "$CONFIG_FILE" 2>/dev/null; then
    echo "ERROR: Generated config has invalid JSON syntax!"
    echo "Restoring backup..."
    sudo cp "$BACKUP_FILE" "$CONFIG_FILE"
    exit 1
  fi
  echo "==> JSON syntax validated"
fi

# Show diff
echo ""
echo "==> Changes made:"
echo "--- BEFORE (backup at $BACKUP_FILE)"
echo "+++ AFTER"
diff -u "$BACKUP_FILE" "$CONFIG_FILE" || true

echo ""
echo "==> Migration complete!"
echo ""
echo "Summary of changes:"
echo "  ❌ Removed: docker_image, docker_bin, gpu_device"
echo "  ✅ Added: ffmpeg_bin, ffprobe_bin"
echo ""
echo "Backup saved to: $BACKUP_FILE"
echo ""
echo "Next steps:"
echo "  1. Review the changes: sudo cat $CONFIG_FILE"
echo "  2. Rebuild daemon: cargo build --release"
echo "  3. Reinstall: sudo install -m 755 target/release/av1d /usr/local/bin/av1d"
echo "  4. Restart service: sudo systemctl restart av1d"
echo ""
