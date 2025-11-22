#!/usr/bin/env bash
set -euo pipefail

# migrate_config_and_service.sh
# Comprehensive migration script for av1d: config + systemd service

CONFIG_FILE="${1:-/etc/av1d/config.json}"
SERVICE_FILE="${2:-/etc/systemd/system/av1d.service}"

echo "╔════════════════════════════════════════════════════════════════╗"
echo "║  AV1 Daemon Migration: Docker → Native FFmpeg                 ║"
echo "╚════════════════════════════════════════════════════════════════╝"
echo ""

# Check if running as root or with sudo
if [[ $EUID -ne 0 ]]; then
   echo "ERROR: This script must be run as root or with sudo"
   echo "Usage: sudo $0 [config-file] [service-file]"
   exit 1
fi

# ============================================================================
# STEP 1: Migrate Configuration File
# ============================================================================

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "STEP 1: Migrate Configuration File"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

if [[ ! -f "$CONFIG_FILE" ]]; then
  echo "ERROR: Config file not found: $CONFIG_FILE"
  exit 1
fi

BACKUP_CONFIG="${CONFIG_FILE}.pre-migration-$(date +%Y%m%d-%H%M%S)"
echo "Creating backup: $BACKUP_CONFIG"
cp "$CONFIG_FILE" "$BACKUP_CONFIG"

# Check if jq is available
if ! command -v jq >/dev/null 2>&1; then
  echo "ERROR: jq is required for safe JSON manipulation"
  echo "Install it with: sudo apt install jq"
  exit 1
fi

# Check if already migrated
if jq -e '.ffmpeg_bin' "$CONFIG_FILE" >/dev/null 2>&1; then
  echo "⚠️  WARNING: Config already has 'ffmpeg_bin' field"
  read -p "Continue anyway? (y/N): " -n 1 -r
  echo
  if [[ ! $REPLY =~ ^[Yy]$ ]]; then
    echo "Aborted."
    exit 0
  fi
fi

# Transform config
echo "Transforming configuration..."
TEMP_CONFIG=$(mktemp)

jq '
  # Remove Docker-related fields
  del(.docker_image, .docker_bin, .gpu_device) |
  
  # Add FFmpeg fields (with defaults)
  .ffmpeg_bin = (.ffmpeg_bin // "ffmpeg") |
  .ffprobe_bin = (.ffprobe_bin // "ffprobe") |
  
  # Add optional new fields (commented out by default)
  # Uncomment these if you want to set them:
  # .require_ffmpeg_version = (.require_ffmpeg_version // "8.0") |
  # .force_reencode = (.force_reencode // false) |
  # .enable_test_clip_workflow = (.enable_test_clip_workflow // true) |
  # .test_clip_duration = (.test_clip_duration // 45) |
  
  # Pretty print
  .
' "$CONFIG_FILE" > "$TEMP_CONFIG"

# Validate generated JSON
if ! jq empty "$TEMP_CONFIG" 2>/dev/null; then
  echo "ERROR: Generated config has invalid JSON!"
  rm "$TEMP_CONFIG"
  exit 1
fi

# Replace original
mv "$TEMP_CONFIG" "$CONFIG_FILE"

echo "✅ Configuration migrated successfully"
echo ""
echo "Changes:"
diff -u "$BACKUP_CONFIG" "$CONFIG_FILE" | grep -E "^[-+]" | head -20 || true
echo ""

# ============================================================================
# STEP 2: Migrate Systemd Service File
# ============================================================================

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "STEP 2: Migrate Systemd Service File"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

if [[ ! -f "$SERVICE_FILE" ]]; then
  echo "⚠️  WARNING: Service file not found: $SERVICE_FILE"
  echo "Skipping service migration."
else
  BACKUP_SERVICE="${SERVICE_FILE}.pre-migration-$(date +%Y%m%d-%H%M%S)"
  echo "Creating backup: $BACKUP_SERVICE"
  cp "$SERVICE_FILE" "$BACKUP_SERVICE"
  
  # Check if already migrated
  if ! grep -q "docker.service" "$SERVICE_FILE"; then
    echo "⚠️  Service file appears already migrated (no docker.service dependency)"
    read -p "Continue anyway? (y/N): " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
      echo "Skipping service migration."
    else
      # Transform service file
      echo "Transforming service file..."
      TEMP_SERVICE=$(mktemp)
      
      # Remove Docker dependencies and add resource limits
      sed -e '/After=docker.service/d' \
          -e '/Requires=docker.service/d' \
          -e 's/Description=AV1 Transcoding Daemon/Description=AV1 Software Transcoding Daemon/' \
          "$SERVICE_FILE" > "$TEMP_SERVICE"
      
      # Add resource limits if not present
      if ! grep -q "CPUQuota" "$TEMP_SERVICE"; then
        # Insert resource limits before [Install] section
        sed -i '/^\[Install\]/i \
# Resource limits for software encoding\
CPUQuota=800%      # Limit to 8 cores (adjust as needed)\
MemoryMax=16G      # Limit to 16GB RAM (adjust as needed)\
Nice=-5            # Higher priority (optional)\
\
' "$TEMP_SERVICE"
      fi
      
      mv "$TEMP_SERVICE" "$SERVICE_FILE"
      
      echo "✅ Service file migrated successfully"
      echo ""
      echo "Changes:"
      diff -u "$BACKUP_SERVICE" "$SERVICE_FILE" | grep -E "^[-+]" | head -20 || true
      echo ""
    fi
  else
    # Transform service file
    echo "Transforming service file..."
    TEMP_SERVICE=$(mktemp)
    
    # Remove Docker dependencies and add resource limits
    sed -e '/After=docker.service/d' \
        -e '/Requires=docker.service/d' \
        -e 's/Description=AV1 Transcoding Daemon/Description=AV1 Software Transcoding Daemon/' \
        "$SERVICE_FILE" > "$TEMP_SERVICE"
    
    # Add resource limits if not present
    if ! grep -q "CPUQuota" "$TEMP_SERVICE"; then
      # Insert resource limits before [Install] section
      sed -i '/^\[Install\]/i \
# Resource limits for software encoding\
CPUQuota=800%      # Limit to 8 cores (adjust as needed)\
MemoryMax=16G      # Limit to 16GB RAM (adjust as needed)\
Nice=-5            # Higher priority (optional)\
\
' "$TEMP_SERVICE"
    fi
    
    mv "$TEMP_SERVICE" "$SERVICE_FILE"
    
    echo "✅ Service file migrated successfully"
    echo ""
    echo "Changes:"
    diff -u "$BACKUP_SERVICE" "$SERVICE_FILE" | grep -E "^[-+]" | head -20 || true
    echo ""
  fi
fi

# ============================================================================
# STEP 3: Summary and Next Steps
# ============================================================================

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "MIGRATION COMPLETE"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo "✅ Configuration migrated: $CONFIG_FILE"
echo "   Backup: $BACKUP_CONFIG"
echo ""
if [[ -f "$SERVICE_FILE" ]]; then
  echo "✅ Service file migrated: $SERVICE_FILE"
  echo "   Backup: $BACKUP_SERVICE"
  echo ""
fi

echo "Summary of changes:"
echo "  ❌ Removed from config:"
echo "     - docker_image"
echo "     - docker_bin"
echo "     - gpu_device"
echo ""
echo "  ✅ Added to config:"
echo "     - ffmpeg_bin: \"ffmpeg\""
echo "     - ffprobe_bin: \"ffprobe\""
echo ""
if [[ -f "$SERVICE_FILE" ]]; then
  echo "  ✅ Updated service file:"
  echo "     - Removed Docker dependencies"
  echo "     - Added CPU/Memory limits"
  echo "     - Updated description"
  echo ""
fi

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "NEXT STEPS"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo "1. Review the migrated configuration:"
echo "   sudo cat $CONFIG_FILE"
echo ""
echo "2. Rebuild the daemon:"
echo "   cargo build --release"
echo ""
echo "3. Reinstall binaries:"
echo "   sudo install -m 755 target/release/av1d /usr/local/bin/av1d"
echo "   sudo install -m 755 target/release/av1top /usr/local/bin/av1top"
echo ""
echo "4. Reload systemd:"
echo "   sudo systemctl daemon-reload"
echo ""
echo "5. Test the daemon (foreground):"
echo "   av1d --config $CONFIG_FILE"
echo "   (Press Ctrl+C to stop)"
echo ""
echo "6. Start the service:"
echo "   sudo systemctl start av1d"
echo ""
echo "7. Monitor with TUI:"
echo "   av1top"
echo ""
echo "8. Check logs:"
echo "   sudo journalctl -u av1d -f"
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo "⚠️  IMPORTANT NOTES:"
echo ""
echo "• Software encoding is 10-20x slower than hardware encoding"
echo "• CPU usage will be near 100% during encoding"
echo "• Adjust CPUQuota in service file if needed (currently 800% = 8 cores)"
echo "• REMUX sources will trigger test clip workflow (requires approval)"
echo "• File sizes may be larger (quality-first approach)"
echo ""
echo "To rollback, restore from backups:"
echo "  sudo cp $BACKUP_CONFIG $CONFIG_FILE"
if [[ -f "$BACKUP_SERVICE" ]]; then
  echo "  sudo cp $BACKUP_SERVICE $SERVICE_FILE"
fi
echo "  sudo systemctl daemon-reload"
echo "  sudo systemctl restart av1d"
echo ""
