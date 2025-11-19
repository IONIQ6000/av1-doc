#!/bin/bash
set -euo pipefail

# AV1 Daemon Installation Script for Debian 13 Trixie
# This script installs the Rust toolchain, builds the project, and installs binaries

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INSTALL_PREFIX="${INSTALL_PREFIX:-/usr/local}"
BIN_DIR="${INSTALL_PREFIX}/bin"
SYSTEMD_DIR="/etc/systemd/system"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Check if running as root for system installation
check_root() {
    if [[ $EUID -ne 0 ]]; then
        log_error "This script must be run as root for system installation"
        log_info "Alternatively, run with INSTALL_PREFIX=\$HOME/.local ./install.sh for user installation"
        exit 1
    fi
}

# Install system dependencies
install_dependencies() {
    log_info "Installing system dependencies..."
    
    apt-get update
    apt-get install -y \
        curl \
        build-essential \
        pkg-config \
        libssl-dev \
        ca-certificates \
        gnupg \
        lsb-release \
        git
    
    log_info "System dependencies installed"
}

# Install Rust toolchain
install_rust() {
    log_info "Checking for Rust installation..."
    
    if command -v rustc &> /dev/null; then
        local rust_version=$(rustc --version)
        log_info "Rust already installed: $rust_version"
    else
        log_info "Installing Rust toolchain..."
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
        
        # Source cargo path for current session
        export PATH="$HOME/.cargo/bin:$PATH"
        source "$HOME/.cargo/env" 2>/dev/null || true
        
        log_info "Rust toolchain installed"
    fi
    
    # Ensure cargo is in PATH
    export PATH="$HOME/.cargo/bin:$PATH"
    if ! command -v cargo &> /dev/null; then
        log_error "Cargo not found in PATH. Please run: source \$HOME/.cargo/env"
        exit 1
    fi
}

# Install Docker (if not already installed)
install_docker() {
    log_info "Checking for Docker installation..."
    
    if command -v docker &> /dev/null; then
        local docker_version=$(docker --version)
        log_info "Docker already installed: $docker_version"
        return
    fi
    
    log_info "Installing Docker..."
    
    # Add Docker's official GPG key
    install -m 0755 -d /etc/apt/keyrings
    curl -fsSL https://download.docker.com/linux/debian/gpg | gpg --dearmor -o /etc/apt/keyrings/docker.gpg
    chmod a+r /etc/apt/keyrings/docker.gpg
    
    # Set up Docker repository
    echo \
      "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.gpg] https://download.docker.com/linux/debian \
      $(. /etc/os-release && echo "$VERSION_CODENAME") stable" | \
      tee /etc/apt/sources.list.d/docker.list > /dev/null
    
    # Install Docker
    apt-get update
    apt-get install -y docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin
    
    # Start and enable Docker service
    systemctl enable docker
    systemctl start docker
    
    log_info "Docker installed and started"
}

# Build the project
build_project() {
    log_info "Building AV1 Daemon project..."
    
    cd "$SCRIPT_DIR"
    
    # Ensure we have the latest Rust toolchain
    rustup update stable
    
    # Build release binaries
    log_info "Compiling release binaries (this may take a while)..."
    cargo build --release
    
    if [[ ! -f "target/release/av1d" ]] || [[ ! -f "target/release/av1top" ]]; then
        log_error "Build failed - binaries not found"
        exit 1
    fi
    
    log_info "Build completed successfully"
}

# Install binaries
install_binaries() {
    log_info "Installing binaries to $BIN_DIR..."
    
    mkdir -p "$BIN_DIR"
    
    install -m 755 target/release/av1d "$BIN_DIR/av1d"
    install -m 755 target/release/av1top "$BIN_DIR/av1top"
    
    log_info "Binaries installed:"
    log_info "  - $BIN_DIR/av1d"
    log_info "  - $BIN_DIR/av1top"
}

# Create systemd service file
create_systemd_service() {
    log_info "Creating systemd service file..."
    
    cat > "$SYSTEMD_DIR/av1d.service" <<EOF
[Unit]
Description=AV1 Transcoding Daemon
After=docker.service network.target
Requires=docker.service

[Service]
Type=simple
User=root
ExecStart=$BIN_DIR/av1d --config /etc/av1d/config.json
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
    
    log_info "Systemd service file created: $SYSTEMD_DIR/av1d.service"
    log_info "To enable and start the service:"
    log_info "  systemctl daemon-reload"
    log_info "  systemctl enable av1d"
    log_info "  systemctl start av1d"
}

# Create default config directory and example config
create_config() {
    log_info "Creating configuration directory..."
    
    mkdir -p /etc/av1d
    
    if [[ ! -f /etc/av1d/config.json ]]; then
        cat > /etc/av1d/config.json <<'EOF'
{
  "library_roots": [
    "/media/movies",
    "/media/tv"
  ],
  "min_bytes": 2147483648,
  "max_size_ratio": 0.90,
  "job_state_dir": "/var/lib/av1d/jobs",
  "scan_interval_secs": 60,
  "docker_image": "lscr.io/linuxserver/ffmpeg:version-8.0-cli",
  "docker_bin": "docker",
  "gpu_device": "/dev/dri"
}
EOF
        log_info "Default configuration created: /etc/av1d/config.json"
        log_warn "Please edit /etc/av1d/config.json with your library paths"
    else
        log_info "Configuration file already exists: /etc/av1d/config.json"
    fi
    
    # Create job state directory
    mkdir -p /var/lib/av1d/jobs
    chmod 755 /var/lib/av1d/jobs
    
    log_info "Job state directory created: /var/lib/av1d/jobs"
}

# Pull Docker image
pull_docker_image() {
    log_info "Pulling Docker ffmpeg image..."
    
    docker pull lscr.io/linuxserver/ffmpeg:version-8.0-cli || {
        log_warn "Failed to pull Docker image. You can pull it manually later with:"
        log_warn "  docker pull lscr.io/linuxserver/ffmpeg:version-8.0-cli"
    }
}

# Main installation function
main() {
    log_info "Starting AV1 Daemon installation..."
    log_info "Install prefix: $INSTALL_PREFIX"
    
    check_root
    install_dependencies
    install_rust
    install_docker
    build_project
    install_binaries
    create_config
    create_systemd_service
    pull_docker_image
    
    log_info ""
    log_info "=========================================="
    log_info "Installation completed successfully!"
    log_info "=========================================="
    log_info ""
    log_info "Next steps:"
    log_info "1. Edit configuration: /etc/av1d/config.json"
    log_info "2. Ensure GPU device is accessible: /dev/dri"
    log_info "3. Enable and start service:"
    log_info "   systemctl daemon-reload"
    log_info "   systemctl enable av1d"
    log_info "   systemctl start av1d"
    log_info "4. Monitor with TUI: av1top"
    log_info "5. Check logs: journalctl -u av1d -f"
    log_info ""
}

# Run main function
main "$@"

