# Dockerfile for building AV1 Daemon in Debian 13 Trixie
FROM debian:trixie-slim

# Install build dependencies
RUN apt-get update && apt-get install -y \
    curl \
    build-essential \
    pkg-config \
    libssl-dev \
    ca-certificates \
    git \
    && rm -rf /var/lib/apt/lists/*

# Install Rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
ENV PATH="/root/.cargo/bin:${PATH}"

# Set working directory
WORKDIR /build

# Copy project files
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

# Build release binaries
RUN cargo build --release

# Create final image with just the binaries
FROM debian:trixie-slim

# Install runtime dependencies (Docker client, etc.)
RUN apt-get update && apt-get install -y \
    docker.io \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Copy binaries from build stage
COPY --from=0 /build/target/release/av1d /usr/local/bin/av1d
COPY --from=0 /build/target/release/av1top /usr/local/bin/av1top

# Create directories
RUN mkdir -p /etc/av1d /var/lib/av1d/jobs

# Copy default config
COPY install.sh /usr/local/bin/install-av1d.sh
RUN chmod +x /usr/local/bin/install-av1d.sh

WORKDIR /var/lib/av1d

# Default command
CMD ["av1d", "--config", "/etc/av1d/config.json"]

