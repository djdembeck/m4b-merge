# =============================================================================
# Build stage - Compile the Rust binary
# =============================================================================
FROM rust:1.96-slim AS builder

WORKDIR /app

# Install build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    pkg-config \
    cmake \
    perl \
    && rm -rf /var/lib/apt/lists/*

# Copy only Cargo files first for better layer caching
COPY Cargo.toml Cargo.lock ./

# Create the src directory structure to prevent build errors
RUN mkdir -p src && echo "fn main() {}" > src/main.rs

# Build dependencies
RUN cargo fetch --locked

# Copy the rest of the source code
COPY . .

# Build the release binary
RUN cargo build --release --locked

# =============================================================================
# Runtime stage - Minimal image with FFmpeg
# =============================================================================
FROM jrottenberg/ffmpeg:6-ubuntu AS runtime

# Install CA certificates and other runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

# Create non-root user
RUN groupadd appgroup && \
    useradd --gid appgroup --shell /bin/bash --create-home appuser

# Create necessary directories
RUN mkdir -p /input /output /config && \
    chown -R appuser:appgroup /input /output /config

# Copy the binary from builder
COPY --from=builder /app/target/release/m4b-merge /usr/local/bin/m4b-merge

# Set environment variables
ENV HOME=/home/appuser

# Switch to non-root user
USER appuser

# Set working directory
WORKDIR /home/appuser

# Default command
ENTRYPOINT ["m4b-merge"]