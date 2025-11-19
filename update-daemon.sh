#!/bin/bash
# Quick update script to rebuild daemon with logging and update systemd service

set -e

echo "Rebuilding daemon with logging support..."
cargo build --release

echo "Installing updated binaries..."
install -m 755 target/release/av1d /usr/local/bin/av1d
install -m 755 target/release/av1top /usr/local/bin/av1top

echo "Updating systemd service file..."
cat > /etc/systemd/system/av1d.service <<EOF
[Unit]
Description=AV1 Transcoding Daemon
After=docker.service network.target
Requires=docker.service

[Service]
Type=simple
User=root
Environment="RUST_LOG=info"
ExecStart=/usr/local/bin/av1d --config /etc/av1d/config.json
Restart=always
RestartSec=10
StandardOutput=journal
StandardError=journal

# Security settings
NoNewPrivileges=true
PrivateTmp=true

# Resource limits (adjust as needed)
LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
EOF

echo "Reloading systemd..."
systemctl daemon-reload

echo "Restarting service..."
systemctl restart av1d

echo "Done! Check logs with: journalctl -u av1d -f"

