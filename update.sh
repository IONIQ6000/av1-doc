#!/bin/bash
# Update script for AV1 daemon and TUI
# This script pulls latest changes, rebuilds, and updates the installed binaries

set -e  # Exit on error

echo "🔄 Updating AV1 daemon and TUI..."

# Step 1: Stop the daemon service if running
echo "📦 Stopping av1d service..."
sudo systemctl stop av1d || echo "Service not running or doesn't exist"

# Step 2: Pull latest changes
echo "⬇️  Pulling latest changes from git..."
cd "$(dirname "$0")"
git pull

# Step 3: Build release binaries
echo "🔨 Building release binaries..."
cargo build --release

# Step 4: Copy binaries to /usr/local/bin
echo "📋 Installing binaries..."
sudo cp target/release/av1d /usr/local/bin/av1d
sudo cp target/release/av1top /usr/local/bin/av1top

# Step 5: Restart the daemon service
echo "🚀 Starting av1d service..."
sudo systemctl start av1d

# Step 6: Show status
echo "✅ Update complete!"
echo ""
echo "Daemon status:"
sudo systemctl status av1d --no-pager -l || true
echo ""
echo "To view logs: journalctl -u av1d -f"
echo "To run TUI: av1top"

